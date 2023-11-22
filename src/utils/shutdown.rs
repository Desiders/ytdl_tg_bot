use crate::config::YtDlp;

use telers::{errors::HandlerError, event::simple::HandlerResult};
use tracing::{event, Level};

#[allow(clippy::module_name_repetitions)]
pub async fn on_shutdown(yt_dlp_config: YtDlp) -> HandlerResult {
    if !yt_dlp_config.remove_on_shutdown {
        return Ok(());
    }

    event!(Level::DEBUG, "Removing yt-dlp");

    tokio::fs::remove_dir_all(yt_dlp_config.dir_path).await.map_err(|err| {
        event!(Level::ERROR, %err, "Error while removing yt-dlp path");

        HandlerError::new(err)
    })?;

    Ok(())
}
