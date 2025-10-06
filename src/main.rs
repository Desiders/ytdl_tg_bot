mod config;
mod database;
mod download;
mod entities;
mod errors;
mod filters;
mod handlers;
mod handlers_utils;
mod interactors;
mod middlewares;
mod services;
mod utils;
mod value_objects;

use config::DatabaseConfig;
use filters::{is_via_bot, text_contains_url, text_contains_url_with_reply, text_empty, url_is_blacklisted, url_is_skippable_by_param};
use froodi::{
    async_impl::{Container, RegistryBuilder},
    instance,
    DefaultScope::{App, Request},
    Inject, InstantiateErrorKind,
};
use handlers::{
    audio_download, media_download_chosen_inline_result, media_download_search_chosen_inline_result, media_search_inline_query,
    media_select_inline_query, start, video_download_quite,
};
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use services::get_cookies_from_directory;
use std::borrow::Cow;
use telers::{
    client::{
        telegram::{APIServer, BareFilesPathWrapper},
        Reqwest,
    },
    enums::{ChatType as ChatTypeEnum, ContentType as ContentTypeEnum},
    filters::{ChatType, Command, ContentType, Filter as _},
    Bot, Dispatcher, Router,
};
use tempfile::env::temp_dir;
use tracing::{event, Level};
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};
use utils::{on_shutdown, on_startup};

use crate::{
    config::{YtDlpConfig, YtPotProviderConfig},
    database::TxManager,
    entities::Cookies,
    handlers::video_download_nw,
    interactors::{
        download::{DownloadVideo, DownloadVideoPlaylist},
        send_media::{SendVideoInFS, SendVideoPlaylistById},
        GetMediaInfo,
    },
    middlewares::ContainerMiddleware,
};

fn init_container(
    bot: Bot,
    yt_dlp_cfg: YtDlpConfig,
    yt_pot_provider_cfg: YtPotProviderConfig,
    cookies: Cookies,
    database_cfg: DatabaseConfig,
) -> Container {
    let registry = RegistryBuilder::new()
        .provide(instance(bot), App)
        .provide(instance(yt_dlp_cfg), App)
        .provide(instance(yt_pot_provider_cfg), App)
        .provide(instance(cookies), App)
        .provide(instance(database_cfg), App)
        .provide(
            |Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
             Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
             Inject(cookies): Inject<Cookies>| { Ok(GetMediaInfo::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies)) },
            Request,
        )
        .provide(
            |Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
             Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
             Inject(cookies): Inject<Cookies>| Ok(DownloadVideo::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies, temp_dir())),
            Request,
        )
        .provide(
            |Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
             Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
             Inject(cookies): Inject<Cookies>| { Ok(DownloadVideoPlaylist::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies)) },
            Request,
        )
        .provide(|Inject(bot): Inject<Bot>| Ok(SendVideoInFS::new(bot)), Request)
        .provide(|Inject(bot): Inject<Bot>| Ok(SendVideoPlaylistById::new(bot)), Request)
        .provide_async(
            |Inject(database_cfg): Inject<DatabaseConfig>| async move {
                let mut options = ConnectOptions::new(database_cfg.get_postgres_url());
                options.sqlx_logging(true);

                match Database::connect(options).await {
                    Ok(database_conn) => {
                        event!(Level::DEBUG, "Database conn created");
                        Ok(database_conn)
                    }
                    Err(err) => {
                        event!(Level::ERROR, %err, "Error creating database conn");
                        Err(InstantiateErrorKind::Custom(err.into()))
                    }
                }
            },
            App,
        )
        .provide_async(
            |Inject(pool): Inject<DatabaseConnection>| async move { Ok(TxManager::new(pool)) },
            Request,
        );
    Container::new(registry)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("{}", &*config::get_path());

    let config = config::parse_from_fs(&*config::get_path()).unwrap();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::builder().parse_lossy(config.logging.dirs))
        .init();

    event!(Level::DEBUG, "Config loaded");

    let cookies = get_cookies_from_directory(&*config.yt_dlp.cookies_path).unwrap_or_default();

    event!(Level::DEBUG, hosts = ?cookies.get_hosts(), "Cookies loaded");

    let base_url = format!("{}/bot{{token}}/{{method_name}}", config.telegram_bot_api.url);
    let files_url = format!("{}/file{{token}}/{{path}}", config.telegram_bot_api.url);

    let bot = Bot::with_client(
        config.bot.token.clone(),
        Reqwest::default().with_api_server(Cow::Owned(APIServer::new(&base_url, &files_url, true, BareFilesPathWrapper))),
    );

    let container = init_container(
        bot.clone(),
        config.yt_dlp.clone(),
        config.yt_pot_provider.clone(),
        cookies.clone(),
        config.database.clone(),
    );

    let mut router = Router::new("main");
    router.telegram_observers_mut().iter_mut().for_each(|observer| {
        observer.inner_middlewares.register(ContainerMiddleware {
            container: container.clone(),
        })
    });
    router.message.register(start).filter(Command::many(["start", "help"]));
    router
        .message
        .register(video_download_nw)
        .filter(ContentType::one(ContentTypeEnum::Text))
        .filter(Command::many(["vd", "video_download"]))
        .filter(text_contains_url_with_reply);
    router
        .message
        .register(audio_download)
        .filter(ContentType::one(ContentTypeEnum::Text))
        .filter(Command::many(["ad", "audio_download"]))
        .filter(text_contains_url_with_reply);
    router
        .message
        .register(video_download_nw)
        .filter(ChatType::one(ChatTypeEnum::Private))
        .filter(text_contains_url_with_reply)
        .filter(is_via_bot.invert());
    router
        .message
        .register(video_download_quite)
        .filter(text_contains_url)
        .filter(url_is_blacklisted.invert())
        .filter(url_is_skippable_by_param.invert())
        .filter(is_via_bot.invert());
    router.inline_query.register(media_select_inline_query).filter(text_contains_url);
    router.inline_query.register(media_search_inline_query).filter(text_empty.invert());
    router
        .chosen_inline_result
        .register(media_download_chosen_inline_result)
        .filter(text_contains_url);
    router
        .chosen_inline_result
        .register(media_download_search_chosen_inline_result)
        .filter(text_empty.invert());

    router.startup.register(on_startup, (bot.clone(),));
    router.shutdown.register(on_shutdown, ());

    let dispatcher = Dispatcher::builder()
        .allowed_updates(router.resolve_used_update_types())
        .main_router(router.configure_default())
        .bot(bot)
        .extension(config.yt_dlp)
        .extension(config.bot)
        .extension(config.yt_toolkit)
        .extension(config.yt_pot_provider)
        .extension(config.chat)
        .extension(config.blacklisted)
        .extension(cookies)
        .build();

    match dispatcher.run_polling().await {
        Ok(()) => {
            event!(Level::INFO, "Bot stopped");
        }
        Err(err) => {
            event!(Level::ERROR, error = %err, "Bot stopped");
        }
    }

    container.close().await;
}
