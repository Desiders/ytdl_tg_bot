use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
    path::Path,
    process::Stdio,
};
use tracing::instrument;

/// Merge the video and audio streams into a single file.
/// # Errors
/// Returns [`io::Error`] if the spawn child process fails.
/// # Returns
/// Returns the child process
#[instrument(skip_all, fields(video_fd = video_fd.as_raw_fd(), audio_fd = audio_fd.as_raw_fd(), output_path = %output_path.as_ref().as_os_str().to_string_lossy()))]
pub fn merge_streams(
    video_fd: OwnedFd,
    audio_fd: OwnedFd,
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
pub async fn convert_to_jpg(input_url: impl AsRef<str>, output_path: impl AsRef<Path>) -> Result<tokio::process::Child, Error> {
    let input_url = input_url.as_ref();
    let output_path = output_path.as_ref();

    tokio::process::Command::new("/usr/bin/ffmpeg")
        .args(&["-y", "-hide_banner", "-loglevel", "error", "-i", input_url])
        .arg(output_path.to_string_lossy().as_ref())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(Into::into)
}
