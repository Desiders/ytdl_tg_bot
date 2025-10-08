use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::time::timeout;
use tracing::{event, instrument, Level};

use crate::utils::format_error_report;

/// Merge the video and audio streams into a single file.
/// # Errors
/// Returns [`io::Error`] if the spawn child process fails.
/// # Returns
/// Returns the child process
#[instrument(skip_all, fields(video_fd = video_fd.as_raw_fd(), audio_fd = audio_fd.as_raw_fd(), output_path = %output_path.as_ref().as_os_str().to_string_lossy()))]
pub fn merge_streams(
    video_fd: &OwnedFd,
    audio_fd: &OwnedFd,
    extension: impl AsRef<str>,
    output_path: impl AsRef<Path>,
) -> Result<tokio::process::Child, io::Error> {
    tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            &format!("pipe:{}", video_fd.as_raw_fd()),
            "-i",
            &format!("pipe:{}", audio_fd.as_raw_fd()),
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "copy",
            "-c:a",
            "copy",
            "-shortest",
            "-nostats",
            "-preset",
            "ultrafast",
            "-f",
            extension.as_ref(),
            output_path.as_ref().to_string_lossy().as_ref(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Convert image to `jpg`
/// # Errors
/// Returns [`io::Error`] if the spawn child process fails.
/// # Returns
/// Returns the child process
#[instrument(skip_all)]
async fn convert_to_jpg(input_url: impl AsRef<str>, output_path: impl AsRef<Path>) -> Result<tokio::process::Child, Error> {
    let input_url = input_url.as_ref();
    let output_path = output_path.as_ref();

    tokio::process::Command::new("/usr/bin/ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i", input_url])
        .arg(output_path.to_string_lossy().as_ref())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(Into::into)
}

#[instrument(skip(temp_dir_path), fields(url = url.as_ref(), id = id.as_ref()))]
pub async fn download_thumbnail_to_path(url: impl AsRef<str>, id: impl AsRef<str>, temp_dir_path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = temp_dir_path.as_ref().join(format!("{}.jpg", id.as_ref()));

    match convert_to_jpg(url, &path).await {
        Ok(mut child) => match timeout(Duration::from_secs(10), child.wait()).await {
            Ok(Ok(status)) => {
                if status.success() {
                    Some(path)
                } else {
                    None
                }
            }
            Ok(Err(err)) => {
                event!(Level::ERROR, err = format_error_report(&err), "Failed to convert thumbnail");
                None
            }
            Err(_) => {
                event!(Level::WARN, "Convert thumbnail timed out");
                None
            }
        },
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Failed to convert thumbnail");
            None
        }
    }
}
