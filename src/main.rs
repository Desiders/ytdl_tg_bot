mod config;
mod download;
mod errors;
mod filters;
mod handlers;
mod handlers_utils;
mod models;
mod services;
mod utils;

use filters::{is_via_bot, text_contains_url, text_contains_url_with_reply, text_empty};
use handlers::{
    audio_download, media_download_chosen_inline_result, media_download_search_chosen_inline_result, media_search_inline_query,
    media_select_inline_query, start, video_download, video_download_quite,
};
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
use tracing::{event, Level};
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};
use utils::{on_shutdown, on_startup};

use crate::services::get_cookies_from_directory;

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

    let mut router = Router::new("main");
    router.message.register(start).filter(Command::many(["start", "help"]));
    router
        .message
        .register(video_download)
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
        .register(video_download)
        .filter(ChatType::one(ChatTypeEnum::Private))
        .filter(text_contains_url_with_reply)
        .filter(is_via_bot.invert());
    router
        .message
        .register(video_download_quite)
        .filter(text_contains_url)
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
}
