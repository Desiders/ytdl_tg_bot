use crate::models::Videos;

use futures_util::stream::StreamExt as _;
use serde_json::{json, Value};
use std::{
    io,
    process::{Output, Stdio},
};
use tokio::process::Command;
use tokio_util::codec::{FramedRead, LinesCodec};
use tracing::{event, Level};

#[derive(Debug, thiserror::Error)]
pub enum GetInfoError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Line codec error: {0}")]
    Line(#[from] tokio_util::codec::LinesCodecError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub async fn download_video_to_path(
    executable_path: &str,
    dir_path: &str,
    id_or_url: &str,
    format: &str,
    output_extension: &str,
) -> Result<(), GetInfoError> {
    let args = &[
        "--no-call-home",
        "--no-check-certificate",
        "--no-cache-dir",
        "--no-mtime",
        "--abort-on-error",
        "--prefer-ffmpeg",
        "--no-simulate",
        "--no-progress",
        "--hls-prefer-ffmpeg",
        "--socket-timeout",
        "15",
        "-o",
        "%(id)s.%(ext)s",
        "-P",
        dir_path,
        "-J",
        id_or_url,
        "-f",
        format,
        "--merge-output-format",
        output_extension,
    ];

    let Output { status, stderr, .. } = Command::new(executable_path).args(args).output().await?;

    if !status.success() {
        let msg = String::from_utf8_lossy(&stderr);

        return Err(io::Error::new(io::ErrorKind::Other, format!("Youtube-dl exited with status `{status}`: {msg}")).into());
    }

    Ok(())
}

pub async fn get_video_or_playlist_info(executable_path: &str, id_or_url: &str) -> Result<Videos, GetInfoError> {
    let args = &[
        "--no-call-home",
        "--no-check-certificate",
        "--skip-download",
        "--abort-on-error",
        "--socket-timeout",
        "15",
        "-o",
        "%(id)s.%(ext)s",
        "-J",
        id_or_url,
    ];

    let mut child = Command::new(executable_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut videos = Vec::new();
    let mut stdout = FramedRead::new(child.stdout.take().unwrap(), LinesCodec::new());
    let mut stderr = FramedRead::new(child.stderr.take().unwrap(), LinesCodec::new());

    while let Some(line) = stdout.next().await {
        let line = line?;

        let value: Value = serde_json::from_reader(line.as_bytes())?;

        let is_playlist = value["_type"] == json!("playlist");

        if is_playlist {
            let Some(entries) = value["entries"].as_array() else {
                continue;
            };

            for entry in entries {
                videos.push(serde_json::from_value(entry.clone())?);
            }
        } else {
            videos.push(serde_json::from_value(value)?);
        }
    }

    let mut lines = vec![];

    while let Some(line) = stderr.next().await {
        let line = line?;

        lines.push(line);
    }

    let status = child.wait().await?;

    if !status.success() {
        event!(Level::ERROR, "Child process exited with error status: {status}");

        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Youtube-dl exited with status `{status}`: {msg}", msg = lines.join("\n")),
        )
        .into());
    }

    Ok(Videos(videos.into()))
}
