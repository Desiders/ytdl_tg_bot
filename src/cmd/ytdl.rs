use crate::{
    handlers_utils::range::Range,
    models::{VideoInYT, VideosInYT},
};

use serde::de::Error as _;
use serde_json::{json, Value};
use std::{
    io::{self, Read},
    os::fd::OwnedFd,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tracing::{event, Level};
use wait_timeout::ChildExt as _;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Download stream to a pipe.
/// This function forks a child process and executes `yt-dl` in it.
/// The child process redirects its stdout to the pipe.
/// # Errors
/// Returns [`io::Error`] if the spawn child process fails
/// # Returns
/// Returns the child process
pub fn download_to_pipe(
    fd: OwnedFd,
    executable_path: impl AsRef<str>,
    url: impl AsRef<str>,
    format: impl AsRef<str>,
) -> Result<Child, io::Error> {
    let args = [
        "--ignore-config",
        "--no-colors",
        "--socket-timeout",
        "5",
        "--output",
        "-",
        "--no-playlist",
        "--no-mtime",
        "--no-write-comments",
        "--quiet",
        "--no-simulate",
        "--no-progress",
        "--no-check-formats",
        "--extractor-args",
        "\
        youtube:player_client=default;player_skip=configs,js;\
        youtubetab:skip=webpage;\
        ",
        "--http-chunk-size",
        "10M",
        "-f",
        format.as_ref(),
        "--",
        url.as_ref(),
    ];

    Command::new(executable_path.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(fd))
        .stderr(Stdio::null())
        .spawn()
}

pub async fn download_video_to_path(
    executable_path: impl AsRef<str>,
    url: impl AsRef<str>,
    format: impl AsRef<str>,
    output_dir_path: impl AsRef<Path>,
    timeout: u64,
    download_thumbnails: bool,
) -> Result<(), io::Error> {
    let output_dir_path = output_dir_path.as_ref().to_string_lossy();

    let mut args = vec![
        "--no-update",
        "--ignore-config",
        "--no-colors",
        "--socket-timeout",
        "5",
        "--paths",
        output_dir_path.as_ref(),
        "--output",
        "%(id)s.%(ext)s",
        "--no-playlist",
        "--no-mtime",
        "--no-write-comments",
        "--quiet",
        "--no-simulate",
        "--no-progress",
        "--no-check-formats",
        "--concurrent-fragments",
        "4",
        "--http-chunk-size",
        "15M",
        "-f",
        format.as_ref(),
    ];

    if download_thumbnails {
        args.push("--write-all-thumbnail");
    }

    args.push("--");
    args.push(url.as_ref());

    let mut child = tokio::process::Command::new(executable_path.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;

    match tokio::time::timeout(Duration::from_secs(timeout), child.wait()).await {
        Ok(Ok(exit_code)) => {
            if exit_code.success() {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("Youtube-dl exited with status `{exit_code}`"),
                ))
            }
        }
        Ok(Err(err)) => Err(err),
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "Youtube-dl timed out")),
    }
}

pub async fn download_audio_to_path(
    executable_path: impl AsRef<str>,
    url: impl AsRef<str>,
    format: impl AsRef<str>,
    output_extension: impl AsRef<str>,
    output_dir_path: impl AsRef<Path>,
    timeout: u64,
    download_thumbnails: bool,
) -> Result<(), io::Error> {
    let output_dir_path = output_dir_path.as_ref().to_string_lossy();

    let mut args = vec![
        "--no-update",
        "--ignore-config",
        "--no-color",
        "--socket-timeout",
        "5",
        "--paths",
        output_dir_path.as_ref(),
        "--output",
        "%(id)s.%(ext)s",
        "--prefer-ffmpeg",
        "--hls-prefer-ffmpeg",
        "--extract-audio",
        "--audio-format",
        output_extension.as_ref(),
        "--no-playlist",
        "--no-mtime",
        "--no-write-comments",
        "--quiet",
        "--no-simulate",
        "--no-progress",
        "--no-check-formats",
        "--concurrent-fragments",
        "4",
        "--http-chunk-size",
        "15M",
        "-f",
        format.as_ref(),
    ];

    if download_thumbnails {
        args.push("--write-all-thumbnail");
    }

    args.push("--");
    args.push(url.as_ref());

    let mut child = tokio::process::Command::new(executable_path.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;

    match tokio::time::timeout(Duration::from_secs(timeout), child.wait()).await {
        Ok(Ok(exit_code)) => {
            if exit_code.success() {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("Youtube-dl exited with status `{exit_code}`"),
                ))
            }
        }
        Ok(Err(err)) => Err(err),
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "Youtube-dl timed out")),
    }
}

pub fn get_media_or_playlist_info(
    executable_path: impl AsRef<str>,
    url: impl AsRef<str>,
    allow_playlist: bool,
    timeout: u64,
    range: &Range,
) -> Result<VideosInYT, Error> {
    let args = [
        "--no-update",
        "--ignore-config",
        "--no-color",
        "--socket-timeout",
        "5",
        "--output",
        "%(id)s.%(ext)s",
        if allow_playlist { "--yes-playlist" } else { "--no-playlist" },
        "--no-mtime",
        "--no-write-comments",
        "--no-write-thumbnail",
        "--quiet",
        "--skip-download",
        "--simulate",
        "--no-progress",
        "--no-check-formats",
        "--concurrent-fragments",
        "4",
        "-I",
        &range.to_range_string(),
        "-J",
        "--",
        url.as_ref(),
    ];

    let mut child = Command::new(executable_path.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let mut stdout = vec![];
    let child_stdout = child.stdout.take();
    io::copy(&mut child_stdout.unwrap(), &mut stdout)?;

    let Some(exit_code) = child.wait_timeout(Duration::from_secs(timeout))? else {
        event!(Level::ERROR, "Child process timed out");

        child.kill()?;

        return Err(io::Error::new(io::ErrorKind::TimedOut, "Youtube-dl timed out").into());
    };

    if !exit_code.success() {
        event!(Level::ERROR, "Child process exited with error status: {exit_code}");

        return Err(io::Error::new(io::ErrorKind::Other, format!("Youtube-dl exited with status `{exit_code}`")).into());
    }

    let mut stderr = vec![];
    if let Some(mut reader) = child.stderr {
        reader.read_to_end(&mut stderr)?;
    }

    let value: Value = serde_json::from_slice(&stdout)?;

    if value["_type"] == json!("playlist") {
        let mut videos = vec![];

        let entries = value["entries"].as_array().ok_or(serde_json::Error::custom("No entries found"))?;

        for entry in entries {
            videos.push(serde_json::from_value(entry.clone())?);
        }

        Ok(VideosInYT::new(videos))
    } else {
        let video: VideoInYT = serde_json::from_value(value)?;

        Ok(VideosInYT::new(vec![video]))
    }
}
