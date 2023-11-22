mod cmd;
mod config;
mod errors;
mod extractors;
mod filters;
mod fs;
mod handlers;
mod handlers_utils;
mod middlewares;
mod models;
mod utils;

use config::read_config_from_env;
use filters::text_contains_url;
use handlers::{audio_download, media_download_chosen_inline_result, media_select_inline_query, start, video_download};
use middlewares::Config as ConfigMiddleware;
use telers::{
    enums::{ChatType as ChatTypeEnum, ContentType as ContentTypeEnum},
    event::ToServiceProvider as _,
    filters::{ChatType, Command, ContentType},
    Bot, Dispatcher, Router,
};
use tracing::{event, Level};
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};
use utils::{get_phantom_audio_id, get_phantom_video_id, on_shutdown, on_startup};

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

            std::process::exit(1);
        }
    };

    let Ok(bot_token) = std::env::var("BOT_TOKEN") else {
        panic!("BOT_TOKEN env variable is not set!");
    };

    let bot = Bot::new(bot_token);

    let mut router = Router::new("main");
    router.message.register(start).filter(Command::many(["start", "help"]));
    router
        .message
        .register(video_download)
        .filter(ContentType::one(ContentTypeEnum::Text))
        .filter(Command::many(["vd", "video_download"]))
        .filter(text_contains_url);
    router
        .message
        .register(audio_download)
        .filter(ContentType::one(ContentTypeEnum::Text))
        .filter(Command::many(["ad", "audio_download"]))
        .filter(text_contains_url);
    router
        .message
        .register(video_download)
        .filter(ContentType::one(ContentTypeEnum::Text))
        .filter(ChatType::one(ChatTypeEnum::Private))
        .filter(text_contains_url);
    router.inline_query.register(media_select_inline_query).filter(text_contains_url);
    router
        .chosen_inline_result
        .register(media_download_chosen_inline_result)
        .filter(text_contains_url);

    let phantom_video_id = match get_phantom_video_id(bot.clone(), config.bot.clone(), config.phantom_video).await {
        Ok(id) => id,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting phantom video id");

            std::process::exit(1);
        }
    };

    let phantom_audio_id = match get_phantom_audio_id(bot.clone(), config.bot.clone(), config.phantom_audio).await {
        Ok(id) => id,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting phantom audio id");

            std::process::exit(1);
        }
    };

    router.update.outer_middlewares.register(ConfigMiddleware::new(
        config.yt_dlp.clone(),
        config.bot,
        phantom_video_id,
        phantom_audio_id,
    ));

    router.startup.register(on_startup, (config.yt_dlp.clone(),));
    router.shutdown.register(on_shutdown, (config.yt_dlp,));

    let dispatcher = Dispatcher::builder()
        .allowed_updates(router.resolve_used_update_types())
        .main_router(router)
        .bot(bot)
        .build();

    match dispatcher.to_service_provider_default().unwrap().run_polling().await {
        Ok(()) => {
            event!(Level::INFO, "Bot stopped");
        }
        Err(err) => {
            event!(Level::ERROR, error = %err, "Bot stopped");
        }
    }
}
