mod cmd;
mod config;
mod download;
mod errors;
mod filters;
mod fs;
mod handlers;
mod handlers_utils;
mod middlewares;
mod models;
mod utils;

use config::read_config_from_env;
use filters::{is_via_bot, text_contains_url, text_contains_url_with_reply};
use handlers::{
    audio_download, media_download_chosen_inline_result, media_select_inline_query, start, video_download, video_download_quite,
};
use middlewares::Config as ConfigMiddleware;
use std::{borrow::Cow, process};
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

#[cfg(not(target_family = "unix"))]
fn main() {
    panic!(
        "This bot can only be run on Unix systems. \
        This is because it uses Unix pipes to communicate between the yt-dl process and the ffmpeg process."
    );
}

#[cfg(target_family = "unix")]
#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let config = match read_config_from_env() {
        Ok(config) => {
            tracing_subscriber::registry()
                .with(fmt::layer())
                .with(EnvFilter::from_env("LOGGING_LEVEL"))
                .init();

            event!(Level::DEBUG, "Config loaded from env");

            config
        }
        Err(err) => {
            eprintln!("Error reading config from env: {err}");

            process::exit(1);
        }
    };

    let base_url = format!("{}/bot{{token}}/{{method_name}}", config.bot.telegram_bot_api_url);
    let files_url = format!("{}/file{{token}}/{{path}}", config.bot.telegram_bot_api_url);

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
    router
        .chosen_inline_result
        .register(media_download_chosen_inline_result)
        .filter(text_contains_url);

    router
        .update
        .outer_middlewares
        .register(ConfigMiddleware::new(config.yt_dlp.clone(), config.bot));

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
}
