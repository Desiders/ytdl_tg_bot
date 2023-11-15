use crate::models::Videos;

use serde_json::{json, Value};
use std::{io, process::Stdio};
use tokio::process::{ChildStderr, ChildStdout, Command};
use tokio_stream::StreamExt as _;
use tokio_util::codec::{FramedRead, LinesCodec};

#[derive(Debug, thiserror::Error)]
pub enum GetInfoError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Line codec error: {0}")]
    Line(#[from] tokio_util::codec::LinesCodecError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub struct OutputSreams {
    pub stdout: FramedRead<ChildStdout, LinesCodec>,
    pub stderr: FramedRead<ChildStderr, LinesCodec>,
}

async fn run_process<'a>(executable_path: &'a str, args: &'a [&str]) -> Result<OutputSreams, io::Error> {
    let cmd = Command::new(executable_path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = FramedRead::new(cmd.stdout.unwrap(), LinesCodec::new());
    let stderr = FramedRead::new(cmd.stderr.unwrap(), LinesCodec::new());

    let output_streams = OutputSreams { stdout, stderr };

    Ok(output_streams)
}

pub async fn download_video_to_stdout<'a>(
    executable_path: &'a str,
    video_id_or_url: &'a str,
    format_id: &'a str,
) -> Result<OutputSreams, GetInfoError> {
    let args = &[
        "--no-call-home",
        "--no-cache-dir",
        "--no-check-certificate",
        "--no-mtime",
        "--prefer-ffmpeg",
        "--hls-prefer-ffmpeg",
        "--abort-on-error",
        "--socket-timeout",
        "15",
        "-o",
        "-",
        "-J",
        video_id_or_url,
        "-f",
        format_id,
    ];

    let output_streams = run_process(executable_path, args).await?;

    Ok(output_streams)
}

pub async fn get_video_or_playlist_info<'a>(executable_path: &'a str, video_id_or_url: &'a str) -> Result<Videos, GetInfoError> {
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
        video_id_or_url,
    ];

    let OutputSreams { mut stdout, mut stderr } = run_process(executable_path, args).await?;

    let mut videos = Vec::new();

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

    while let Some(line) = stderr.next().await {
        let line = line?;

        eprintln!("{}", line);
    }

    Ok(Videos(videos.into()))
}
