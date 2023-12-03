use crate::config::YtDlp;

use telers::{errors::HandlerError, event::simple::HandlerResult, methods::SetMyCommands, types::BotCommand, Bot};
use tracing::{event, Level};
use youtube_dl::download_yt_dlp;

async fn set_my_commands(bot: Bot) -> HandlerResult {
    let commands = [
        BotCommand::new("start", "Start the bot"),
        BotCommand::new("vd", "Download a video"),
        BotCommand::new("ad", "Download an audio"),
    ];

    bot.send(SetMyCommands::new(commands)).await?;

    Ok(())
}

#[allow(clippy::module_name_repetitions)]
pub async fn on_startup(bot: Bot, yt_dlp_config: YtDlp) -> HandlerResult {
    let file_exists = tokio::fs::metadata(&yt_dlp_config.full_path)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false);

    if file_exists && !yt_dlp_config.update_on_startup {
        return Ok(());
    }

    event!(Level::DEBUG, ?yt_dlp_config, "Downloading yt-dlp");

    download_yt_dlp(yt_dlp_config.dir_path).await.map_err(|err| {
        event!(Level::ERROR, %err, "Error while downloading yt-dlp path");

        HandlerError::new(err)
    })?;

    set_my_commands(bot).await
}
