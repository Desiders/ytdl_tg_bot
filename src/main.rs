mod cmd;
mod config;
mod errors;
mod extractors;
mod filters;
mod handlers;
mod middlewares;
mod models;

use config::{read_config_from_env, Bot as BotConfig, PhantomVideo as PhantomVideoConfig, PhantomVideoId, YtDlp as YtDlpConfig};
use filters::text_contains_url;
use handlers::{start, video_download, video_download_chosen_inline_result, video_select_inline_query};
use middlewares::Config as ConfigMiddleware;
use telers::{
    enums::{ChatType as ChatTypeEnum, ContentType as ContentTypeEnum},
    errors::{HandlerError, SessionErrorKind},
    event::{simple, ToServiceProvider as _},
    filters::{ChatType, Command, ContentType},
    methods::{DeleteMessage, SendVideo},
    types::InputFile,
    Bot, Dispatcher, Router,
};
use tracing::{event, Level};
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};
use youtube_dl::download_yt_dlp;

async fn on_startup(yt_dlp_config: YtDlpConfig) -> simple::HandlerResult {
    event!(Level::DEBUG, ?yt_dlp_config, "Downloading yt-dlp");

    let file_exists = tokio::fs::metadata(&yt_dlp_config.full_path)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false);

    if file_exists && !yt_dlp_config.update_on_startup {
        return Ok(());
    }

    download_yt_dlp(yt_dlp_config.dir_path).await.map_err(|err| {
        event!(Level::ERROR, %err, "Error while downloading yt-dlp path");

        HandlerError::new(err)
    })?;

    Ok(())
}

async fn on_shutdown(yt_dlp_config: YtDlpConfig) -> simple::HandlerResult {
    if !yt_dlp_config.remove_on_shutdown {
        return Ok(());
    }

    tokio::fs::remove_dir_all(yt_dlp_config.dir_path).await.map_err(|err| {
        event!(Level::ERROR, %err, "Error while removing yt-dlp path");

        HandlerError::new(err)
    })?;

    Ok(())
}

async fn get_phantom_video_id(
    bot: Bot,
    bot_config: BotConfig,
    phantom_video_config: PhantomVideoConfig,
) -> Result<PhantomVideoId, SessionErrorKind> {
    match phantom_video_config {
        PhantomVideoConfig::Id(id) => {
            event!(Level::DEBUG, ?id, "Got phantom video id from config");

            Ok(id)
        }
        PhantomVideoConfig::Path(path) => {
            event!(Level::DEBUG, ?path, "Got phantom video path from config");

            let phantom_file = InputFile::fs(path);

            event!(Level::DEBUG, ?phantom_file, "Sending phantom video");

            let message = bot
                .send(SendVideo::new(bot_config.receiver_video_chat_id, phantom_file).disable_notification(true))
                .await?;

            tokio::spawn(async move {
                bot.send(DeleteMessage::new(bot_config.receiver_video_chat_id, message.message_id))
                    .await
            });

            // `unwrap` is safe because we checked that `message.video` is `Some` in `SendVideo` method
            Ok(PhantomVideoId(message.video.unwrap().file_id.into_string()))
        }
    }
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
        .register(video_download)
        .filter(ContentType::one(ContentTypeEnum::Text))
        .filter(ChatType::one(ChatTypeEnum::Private))
        .filter(text_contains_url);
    router
        .message
        .register(video_download)
        .filter(ContentType::one(ContentTypeEnum::Text))
        .filter(Command::many(["d", "download", "vd", "video_download"]))
        .filter(text_contains_url);
    router.inline_query.register(video_select_inline_query).filter(text_contains_url);
    router
        .chosen_inline_result
        .register(video_download_chosen_inline_result)
        .filter(text_contains_url);

    let phantom_video_id = match get_phantom_video_id(bot.clone(), config.bot.clone(), config.phantom_video).await {
        Ok(id) => id,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting phantom video id");

            std::process::exit(1);
        }
    };

    router
        .update
        .outer_middlewares
        .register(ConfigMiddleware::new(config.yt_dlp.clone(), config.bot, phantom_video_id.clone()));

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
