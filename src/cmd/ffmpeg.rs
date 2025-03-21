use super::get_media_info;
use crate::models::StreamsInfo;

use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
    path::Path,
    process::Stdio,
};
use tracing::{event, instrument, Level};

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
    tokio::process::Command::new("/usr/bin/ffmpeg")
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

/// Convert image to `jpg` with resize.
/// # Errors
/// Returns [`io::Error`] if the spawn child process fails.
/// # Returns
/// Returns the child process
#[instrument(skip_all)]
pub async fn convert_to_jpg(input_url: impl AsRef<str>, output_path: impl AsRef<Path>) -> Result<tokio::process::Child, Error> {
    let input_url = input_url.as_ref();
    let output_path = output_path.as_ref();

    event!(Level::TRACE, input_url);

    let StreamsInfo { streams } = get_media_info(input_url).await.map_err(|err| match err {
        super::ffprobe::Error::Io(err) => Error::Io(err),
        super::ffprobe::Error::Json(err) => Error::Json(err),
    })?;
    let stream = &streams[0];

    event!(Level::TRACE, ?stream, "Stream info");

    let video_width = stream.width.unwrap_or(1280.0) as i64;
    let video_height = stream.height.unwrap_or(720.0) as i64;

    let target_width = 405;
    let target_height = 720;
    let target_aspect = target_width as f64 / target_height as f64;
    let video_aspect = video_width as f64 / video_height as f64;

    event!(
        Level::TRACE,
        "Video: {}x{}, Aspect: {}. Target: {}x{}, Aspect: {}",
        video_width,
        video_height,
        video_aspect,
        target_width,
        target_height,
        target_aspect
    );

    let mut args = vec!["-y", "-hide_banner", "-loglevel", "error", "-i", input_url];

    if (video_aspect - 16.0 / 9.0).abs() < 0.01 {
        let filter = "crop=9/16*ih:ih";
        event!(Level::TRACE, "Applying filter: {}", filter);
        args.push("-vf");
        args.push(filter);
    } else {
        event!(Level::TRACE, "No filter applied");
    }

    tokio::process::Command::new("/usr/bin/ffmpeg")
        .args(args)
        .arg(output_path.to_string_lossy().as_ref())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(Into::into)
}
