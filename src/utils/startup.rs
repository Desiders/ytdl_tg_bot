use crate::config::YtDlp;

use telers::{errors::HandlerError, event::simple::HandlerResult};
use tracing::{event, Level};
use youtube_dl::download_yt_dlp;

#[allow(clippy::module_name_repetitions)]
pub async fn on_startup(yt_dlp_config: YtDlp) -> HandlerResult {
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

    Ok(())
}
