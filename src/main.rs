mod config;
mod errors;
mod extractors;
mod filters;
mod handlers;
mod middlewares;
mod models;

use config::{read_config_from_env, YtDlp};
use filters::is_correct_url;
use handlers::{start, url};
use middlewares::Config as ConfigMiddleware;
use telers::{
    enums::ContentType as ContentTypeEnum,
    errors::HandlerError,
    event::{simple, ToServiceProvider as _},
    filters::{Command, ContentType},
    Bot, Dispatcher, Router,
};
use tracing::{event, Level};
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};
use youtube_dl::download_yt_dlp;

async fn on_startup(yt_dlp: YtDlp) -> simple::HandlerResult {
    let file_exists = tokio::fs::metadata(&yt_dlp.full_path)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false);

    if file_exists && !yt_dlp.update_on_startup {
        return Ok(());
    }

    download_yt_dlp(yt_dlp.dir_path).await.map_err(|err| {
        event!(Level::ERROR, %err, "Error while downloading yt-dlp path");

        HandlerError::new(err)
    })?;

    Ok(())
}

async fn on_shutdown(yt_dlp: YtDlp) -> simple::HandlerResult {
    if !yt_dlp.remove_on_shutdown {
        return Ok(());
    }

    tokio::fs::remove_dir_all(yt_dlp.dir_path).await.map_err(|err| {
        event!(Level::ERROR, %err, "Error while removing yt-dlp path");

        HandlerError::new(err)
    })?;

    Ok(())
}

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
        .register(url)
        .filter(ContentType::one(ContentTypeEnum::Text))
        .filter(is_correct_url);
    router
        .update
        .outer_middlewares
        .register(ConfigMiddleware::new(config.yt_dlp.clone()));

    router.startup.register(on_startup, (config.yt_dlp.clone(),));
    router.shutdown.register(on_shutdown, (config.yt_dlp,));

    let dispatcher = Dispatcher::builder()
        .allowed_updates(router.resolve_used_update_types())
        .main_router(router)
        .bot(bot)
        .build();

    match dispatcher.to_service_provider_default().unwrap().run_polling().await {
        Ok(_) => event!(Level::INFO, "Bot stopped"),
        Err(err) => event!(Level::ERROR, error = %err, "Bot stopped"),
    }
}
