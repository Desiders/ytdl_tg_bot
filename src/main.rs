mod config;
mod database;
mod di_container;
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

use froodi::telers::setup_async_default;
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
use tracing::{error, info};
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};

use crate::{
    filters::{is_via_bot, text_contains_url, text_contains_url_with_reply, text_empty, url_is_blacklisted, url_is_skippable_by_param},
    handlers::{audio, chosen_inline, inline_query, start, video},
    middlewares::{CreateChatMiddleware, ReactionMiddleware, ReplaceDomainsMiddleware},
    services::get_cookies_from_directory,
    utils::{on_shutdown, on_startup},
};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("{}", &*config::get_path());

    let config = config::parse_from_fs(&*config::get_path()).unwrap();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::builder().parse_lossy(config.logging.dirs.as_ref()))
        .init();

    let cookies = get_cookies_from_directory(&*config.yt_dlp.cookies_path).unwrap_or_default();

    info!(hosts = ?cookies.get_hosts(), "Cookies loaded");

    let base_url = format!("{}/bot{{token}}/{{method_name}}", config.telegram_bot_api.url);
    let files_url = format!("{}/file{{token}}/{{path}}", config.telegram_bot_api.url);

    let bot = Bot::with_client(
        config.bot.token.clone(),
        Reqwest::default().with_api_server(Cow::Owned(APIServer::new(&base_url, &files_url, true, BareFilesPathWrapper))),
    );

    let container = di_container::init(bot.clone(), config, cookies);

    let router = Router::new("main");
    let mut router = setup_async_default(router, container.clone());

    router.update.outer_middlewares.register(CreateChatMiddleware);
    router.message.register(start).filter(Command::many(["start", "help"]));

    let mut download_router = Router::new("download");
    download_router.message.inner_middlewares.register(ReplaceDomainsMiddleware);
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
            info!("Bot stopped");
        }
        Err(err) => {
            error!(error = %err, "Bot stopped");
        }
    }

    container.close().await;
}
