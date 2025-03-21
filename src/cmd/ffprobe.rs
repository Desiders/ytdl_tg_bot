use crate::models::StreamsInfo;

use std::{
    io,
    process::{Output, Stdio},
};
use tracing::{event, Level};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub async fn get_media_info(input_url: impl AsRef<str>) -> Result<StreamsInfo, Error> {
    let input_url = input_url.as_ref();

    let Output { status, stdout, stderr } = tokio::process::Command::new("/usr/bin/ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_streams", input_url])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .await?;

    if !status.success() {
        event!(
            Level::ERROR,
            "Child process exited with error status: {:?}. Stderr: {}",
            status.code(),
            String::from_utf8_lossy(stderr.as_slice()),
        );

        return Err(io::Error::new(io::ErrorKind::Other, format!("ffprobe exited with status `{:?}`", status.code())).into());
    }

    serde_json::from_slice(&stdout).map_err(Into::into)
}
