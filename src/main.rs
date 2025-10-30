mod config;
mod database;
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

use crate::{
    config::{Config, DatabaseConfig, YtDlpConfig, YtPotProviderConfig, YtToolkitConfig},
    database::TxManager,
    entities::Cookies,
    filters::{is_via_bot, text_contains_url, text_contains_url_with_reply, text_empty, url_is_blacklisted, url_is_skippable_by_param},
    handlers::{audio, chosen_inline, inline_query, start, video},
    interactors::{
        download::{DownloadAudio, DownloadAudioPlaylist, DownloadVideo, DownloadVideoPlaylist},
        send_media::{
            EditAudioById, EditVideoById, SendAudioById, SendAudioInFS, SendAudioPlaylistById, SendVideoById, SendVideoInFS,
            SendVideoPlaylistById,
        },
        AddDownloadedAudio, AddDownloadedVideo, CreateChat, GetDownloadedMedia, GetMediaInfoById, GetMediaInfoByURL,
        GetShortMediaByURLInfo, SearchMediaInfo,
    },
    middlewares::ReactionMiddleware,
    services::get_cookies_from_directory,
    utils::{on_shutdown, on_startup},
};
use froodi::{
    async_impl::Container,
    async_registry, instance, registry,
    telers::setup_async_default,
    DefaultScope::{App, Request},
    Inject, InstantiateErrorKind,
};
use reqwest::Client;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::{borrow::Cow, sync::Mutex};
use telers::{
    client::{
        telegram::{APIServer, BareFilesPathWrapper},
        Reqwest,
    },
    enums::{ChatType as ChatTypeEnum, ContentType as ContentTypeEnum},
    filters::{ChatType, Command, ContentType, Filter as _},
    Bot, Dispatcher, Router,
};
use tracing::{event, Level};
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};
use uuid::ContextV7;

fn init_container(bot: Bot, config: Config, cookies: Cookies) -> Container {
    let sync_registry = registry! {
        scope(App) [
            provide(instance(bot)),
            provide(instance(cookies)),
            provide(instance(config.bot)),
            provide(instance(config.chat)),
            provide(instance(config.blacklisted)),
            provide(instance(config.logging)),
            provide(instance(config.database)),
            provide(instance(config.yt_dlp)),
            provide(instance(config.yt_toolkit)),
            provide(instance(config.yt_pot_provider)),
            provide(instance(config.telegram_bot_api)),

            provide(|| Ok(Mutex::new(ContextV7::new()))),
            provide(|| Ok(Client::new())),
            provide(|| Ok(GetDownloadedMedia::new())),
            provide(|| Ok(CreateChat::new())),

            provide(|Inject(bot): Inject<Bot>| Ok(SendVideoInFS::new(bot))),
            provide(|Inject(bot): Inject<Bot>| Ok(SendVideoById::new(bot))),
            provide(|Inject(bot): Inject<Bot>| Ok(SendVideoPlaylistById::new(bot))),
            provide(|Inject(bot): Inject<Bot>| Ok(SendAudioInFS::new(bot))),
            provide(|Inject(bot): Inject<Bot>| Ok(SendAudioById::new(bot))),
            provide(|Inject(bot): Inject<Bot>| Ok(SendAudioPlaylistById::new(bot))),
            provide(|Inject(bot): Inject<Bot>| Ok(EditVideoById::new(bot))),
            provide(|Inject(bot): Inject<Bot>| Ok(EditAudioById::new(bot))),
            provide(|Inject(context): Inject<Mutex<ContextV7>>| Ok(AddDownloadedVideo::new(context))),
            provide(|Inject(context): Inject<Mutex<ContextV7>>| Ok(AddDownloadedAudio::new(context))),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>| Ok(GetMediaInfoByURL::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>| Ok(GetMediaInfoById::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies))
            ),
            provide(
                |Inject(client): Inject<Client>,
                Inject(yt_toolkit_cfg): Inject<YtToolkitConfig>| Ok(GetShortMediaByURLInfo::new(client, yt_toolkit_cfg))
            ),
            provide(|
                Inject(client): Inject<Client>,
                Inject(yt_toolkit_cfg): Inject<YtToolkitConfig>| Ok(SearchMediaInfo::new(client, yt_toolkit_cfg))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>| Ok(DownloadVideo::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>| Ok(DownloadVideoPlaylist::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>| Ok(DownloadAudio::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>| Ok(DownloadAudioPlaylist::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies))
            ),
        ],
    };
    let registry_with_sync = async_registry! {
        provide(
            App,
            |Inject(database_cfg): Inject<DatabaseConfig>| async move {
                let mut options = ConnectOptions::new(database_cfg.get_postgres_url());
                options.sqlx_logging(true);

                match Database::connect(options).await {
                    Ok(database_conn) => {
                        event!(Level::INFO, "Database conn created");
                        Ok(database_conn)
                    }
                    Err(err) => {
                        event!(Level::ERROR, %err, "Error creating database conn");
                        Err(InstantiateErrorKind::Custom(err.into()))
                    }
                }
            },
        ),
        provide(
            Request,
            |Inject(pool): Inject<DatabaseConnection>| async move { Ok(TxManager::new(pool)) },
        ),
        extend(sync_registry),
    };

    Container::new(registry_with_sync)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("{}", &*config::get_path());

    let config = config::parse_from_fs(&*config::get_path()).unwrap();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::builder().parse_lossy(config.logging.dirs.as_ref()))
        .init();

    let cookies = get_cookies_from_directory(&*config.yt_dlp.cookies_path).unwrap_or_default();

    event!(Level::INFO, hosts = ?cookies.get_hosts(), "Cookies loaded");

    let base_url = format!("{}/bot{{token}}/{{method_name}}", config.telegram_bot_api.url);
    let files_url = format!("{}/file{{token}}/{{path}}", config.telegram_bot_api.url);

    let bot = Bot::with_client(
        config.bot.token.clone(),
        Reqwest::default().with_api_server(Cow::Owned(APIServer::new(&base_url, &files_url, true, BareFilesPathWrapper))),
    );

    let container = init_container(bot.clone(), config, cookies);

    let router = Router::new("main");
    let mut router = setup_async_default(router, container.clone());

    router.message.register(start).filter(Command::many(["start", "help"]));

    let mut download_router = Router::new("download");
    download_router.message.inner_middlewares.register(ReactionMiddleware);
    download_router
        .message
        .register(video::download)
        .filter(ContentType::one(ContentTypeEnum::Text))
        .filter(Command::many(["vd", "video_download"]))
        .filter(text_contains_url_with_reply);
    download_router
        .message
        .register(audio::download)
        .filter(ContentType::one(ContentTypeEnum::Text))
        .filter(Command::many(["ad", "audio_download"]))
        .filter(text_contains_url_with_reply);
    download_router
        .message
        .register(video::download)
        .filter(ChatType::one(ChatTypeEnum::Private))
        .filter(text_contains_url_with_reply)
        .filter(is_via_bot.invert());
    download_router
        .message
        .register(video::download_quite)
        .filter(text_contains_url)
        .filter(url_is_blacklisted.invert())
        .filter(url_is_skippable_by_param.invert())
        .filter(is_via_bot.invert());
    download_router
        .inline_query
        .register(inline_query::select_by_url)
        .filter(text_contains_url);
    download_router
        .inline_query
        .register(inline_query::select_by_text)
        .filter(text_empty.invert());
    download_router
        .chosen_inline_result
        .register(chosen_inline::download_by_url)
        .filter(text_contains_url);
    download_router
        .chosen_inline_result
        .register(chosen_inline::download_by_id)
        .filter(text_empty.invert());

    router.include(download_router);

    router.startup.register(on_startup, (bot.clone(),));
    router.shutdown.register(on_shutdown, ());

    let dispatcher = Dispatcher::builder()
        .allowed_updates(router.resolve_used_update_types())
        .main_router(router.configure_default())
        .bot(bot)
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
