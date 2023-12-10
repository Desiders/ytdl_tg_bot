use crate::models::VideosInYT;

use futures_util::stream::StreamExt as _;
use serde_json::{json, Value};
use std::{
    io,
    process::{Output, Stdio},
};
use tokio::process::Command;
use tokio_util::codec::{FramedRead, LinesCodec};
use tracing::{event, Level};

const DOWNLOAD_VIDEO_TIMEOUT: u64 = 120; // 2 minutes
const DOWNLOAD_AUDIO_TIMEOUT: u64 = 120; // 2 minutes
const GET_VIDEO_INFO_TIMEOUT: u64 = 60; // 1 minute

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Line codec error: {0}")]
    Line(#[from] tokio_util::codec::LinesCodecError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Timeout error: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
}

pub async fn download_video_to_path(
    executable_path: &str,
    dir_path: &str,
    id_or_url: &str,
    format: &str,
    output_extension: &str,
    allow_playlist: bool,
    download_thumbnails: bool,
) -> Result<(), Error> {
    let mut args = Vec::from([
        "--no-call-home",
        "--no-check-certificate",
        "--no-cache-dir",
        "--no-mtime",
        "--abort-on-error",
        "--prefer-ffmpeg",
        "--hls-prefer-ffmpeg",
        "--no-simulate",
        "--no-progress",
        "--socket-timeout",
        "15",
        if allow_playlist { "--yes-playlist" } else { "--no-playlist" },
        "-o",
        "%(id)s.%(ext)s",
        "-P",
        dir_path,
        "-J",
        "-f",
        format,
        "--merge-output-format",
        output_extension,
        id_or_url,
    ]);

    if download_thumbnails {
        args.push("--write-all-thumbnail");
    }

    let command_task = Command::new(executable_path).args(args).output();
    let timeout_task = tokio::time::sleep(std::time::Duration::from_secs(DOWNLOAD_VIDEO_TIMEOUT));

    tokio::select! {
        result = command_task => {
            let Output { status, stderr, .. } = result?;

            if !status.success() {
                let msg = String::from_utf8_lossy(&stderr);

                return Err(io::Error::new(io::ErrorKind::Other, format!("Youtube-dl exited with status `{status}`: {msg}")).into());
            }

            Ok(())
        },
        _ = timeout_task => {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Youtube-dl timed out").into());
        }
    }
}

pub async fn download_audio_to_path(
    executable_path: &str,
    dir_path: &str,
    id_or_url: &str,
    format: &str,
    output_extension: &str,
    download_thumbnails: bool,
) -> Result<(), Error> {
    let mut args = Vec::from([
        "--no-call-home",
        "--no-check-certificate",
        "--no-cache-dir",
        "--no-mtime",
        "--abort-on-error",
        "--prefer-ffmpeg",
        "--hls-prefer-ffmpeg",
        "--no-simulate",
        "--no-progress",
        "--socket-timeout",
        "15",
        "--extract-audio",
        "--audio-format",
        output_extension,
        "-o",
        "%(id)s.%(ext)s",
        "-P",
        dir_path,
        "-J",
        "-f",
        format,
        id_or_url,
    ]);

    if download_thumbnails {
        args.push("--write-all-thumbnail");
    }

    let command_task = Command::new(executable_path).args(args).output();
    let timeout_task = tokio::time::sleep(std::time::Duration::from_secs(DOWNLOAD_AUDIO_TIMEOUT));

    tokio::select! {
        result = command_task => {
            let Output { status, stderr, .. } = result?;

            if !status.success() {
                let msg = String::from_utf8_lossy(&stderr);

                return Err(io::Error::new(io::ErrorKind::Other, format!("Youtube-dl exited with status `{status}`: {msg}")).into());
            }

            Ok(())
        },
        _ = timeout_task => {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Youtube-dl timed out").into());
        }
    }
}

pub async fn get_video_or_playlist_info(executable_path: &str, id_or_url: &str, allow_playlist: bool) -> Result<VideosInYT, Error> {
    let args = [
        "--no-call-home",
        "--no-check-certificate",
        "--no-cache-dir",
        "--skip-download",
        "--no-simulate",
        "--no-progress",
        "--abort-on-error",
        "--socket-timeout",
        "15",
        if allow_playlist { "--yes-playlist" } else { "--no-playlist" },
        "-o",
        "%(id)s.%(ext)s",
        "-J",
        id_or_url,
    ];

    let command_task = async {
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

                if !allow_playlist && is_playlist {
                    event!(Level::WARN, "Playlist not allowed, but got playlist");

                    if let Some(entry) = entries.iter().next() {
                        videos.push(serde_json::from_value(entry.clone())?);
                    }
                } else {
                    for entry in entries {
                        videos.push(serde_json::from_value(entry.clone())?);
                    }
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

        Ok(VideosInYT::new(videos))
    };
    let timeout_task = tokio::time::sleep(std::time::Duration::from_secs(GET_VIDEO_INFO_TIMEOUT));

    tokio::select! {
        result = command_task => {
            result
        },
        _ = timeout_task => {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Youtube-dl timed out").into());
        }
    }
}
