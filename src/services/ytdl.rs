use crate::{
    handlers_utils::range::Range,
    models::{Cookie, Video, VideosInYT},
};

use serde::de::Error as _;
use serde_json::Value;
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
    pot_provider_api_url: impl AsRef<str>,
    format: impl AsRef<str>,
    cookie: Option<&Cookie>,
) -> Result<Child, io::Error> {
    let mut args = vec![
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
        "--http-chunk-size",
        "10M",
        "-f",
        format.as_ref(),
    ];

    let extractor_arg = format!("youtubepot-bgutilhttp:base_url={}", pot_provider_api_url.as_ref());
    args.push("--extractor-args");
    args.push(&extractor_arg);

    let cookie_path = cookie.map(|c| c.path.to_string_lossy());
    if let Some(cookie_path) = cookie_path.as_deref() {
        event!(Level::TRACE, "Using cookies from: {}", cookie_path);

        args.push("--cookies");
        args.push(cookie_path);
    } else {
        event!(Level::TRACE, "No cookies provided");
    }

    args.push("--");
    args.push(url.as_ref());

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
    pot_provider_api_url: impl AsRef<str>,
    format: impl AsRef<str>,
    output_dir_path: impl AsRef<Path>,
    timeout: u64,
    download_thumbnails: bool,
    cookie: Option<&Cookie>,
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
        "--embed-metadata",
        "--concurrent-fragments",
        "4",
        "--http-chunk-size",
        "10M",
        "-f",
        format.as_ref(),
    ];

    if download_thumbnails {
        args.push("--write-all-thumbnail");
    }

    let extractor_arg = format!("youtubepot-bgutilhttp:base_url={}", pot_provider_api_url.as_ref());
    args.push("--extractor-args");
    args.push(&extractor_arg);

    let cookie_path = cookie.map(|c| c.path.to_string_lossy());
    if let Some(cookie_path) = cookie_path.as_deref() {
        event!(Level::TRACE, "Using cookies from: {}", cookie_path);

        args.push("--cookies");
        args.push(cookie_path);
    } else {
        event!(Level::TRACE, "No cookies provided");
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
                Err(io::Error::other(format!("Youtube-dl exited with status `{exit_code}`")))
            }
        }
        Ok(Err(err)) => Err(err),
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "Youtube-dl timed out")),
    }
}

pub async fn download_audio_to_path(
    executable_path: impl AsRef<str>,
    url: impl AsRef<str>,
    pot_provider_api_url: impl AsRef<str>,
    format: impl AsRef<str>,
    output_extension: impl AsRef<str>,
    output_dir_path: impl AsRef<Path>,
    timeout: u64,
    download_thumbnails: bool,
    cookie: Option<&Cookie>,
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
        "--embed-metadata",
        "--concurrent-fragments",
        "4",
        "--http-chunk-size",
        "10M",
        "-f",
        format.as_ref(),
    ];

    if download_thumbnails {
        args.push("--write-all-thumbnail");
    }

    let extractor_arg = format!("youtubepot-bgutilhttp:base_url={}", pot_provider_api_url.as_ref());
    args.push("--extractor-args");
    args.push(&extractor_arg);
    args.push("--extractor-args");
    args.push("youtube:player_client=default,mweb,web_music,web_creator;player_skip=configs,initial_data");

    let cookie_path = cookie.map(|c| c.path.to_string_lossy());
    if let Some(cookie_path) = cookie_path.as_deref() {
        event!(Level::TRACE, "Using cookies from: {}", cookie_path);

        args.push("--cookies");
        args.push(cookie_path);
    } else {
        event!(Level::TRACE, "No cookies provided");
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
                Err(io::Error::other(format!("Youtube-dl exited with status `{exit_code}`")))
            }
        }
        Ok(Err(err)) => Err(err),
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "Youtube-dl timed out")),
    }
}

pub fn get_media_or_playlist_info(
    executable_path: impl AsRef<str>,
    url_or_id: impl AsRef<str>,
    pot_provider_api_url: impl AsRef<str>,
    allow_playlist: bool,
    timeout: u64,
    range: &Range,
    cookie: Option<&Cookie>,
) -> Result<VideosInYT, Error> {
    let range_string = range.to_range_string();

    let mut args = vec![
        "--no-update",
        "--ignore-config",
        "--no-color",
        "--socket-timeout",
        "5",
        "--output",
        "%(id)s.%(ext)s",
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
        &range_string,
        "-J",
    ];

    let extractor_arg = format!("youtubepot-bgutilhttp:base_url={}", pot_provider_api_url.as_ref());
    args.push("--extractor-args");
    args.push(&extractor_arg);
    args.push("--extractor-args");
    args.push("youtube:player_client=default,mweb,web_music,web_creator;player_skip=configs,initial_data");

    if allow_playlist {
        args.push("--yes-playlist");
    } else {
        args.push("--no-playlist");
    }

    let cookie_path = cookie.map(|c| c.path.to_string_lossy());
    if let Some(cookie_path) = cookie_path.as_deref() {
        event!(Level::TRACE, "Using cookies from: {}", cookie_path);

        args.push("--cookies");
        args.push(cookie_path);
    } else {
        event!(Level::TRACE, "No cookies provided");
    }

    args.push("--");
    args.push(url_or_id.as_ref());

    let mut child = Command::new(executable_path.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdout = vec![];
    let child_stdout = child.stdout.take();
    io::copy(&mut child_stdout.unwrap(), &mut stdout)?;

    let mut stderr = vec![];
    let child_stderr = child.stderr.take();
    if let Some(mut reader) = child_stderr {
        reader.read_to_end(&mut stderr)?;
        if !stderr.is_empty() {
            event!(Level::ERROR, "Youtube-dl stderr: {}", String::from_utf8_lossy(&stderr));
        }
    }

    let Some(status) = child.wait_timeout(Duration::from_secs(timeout))? else {
        event!(Level::ERROR, "Child process timed out");
        child.kill()?;
        return Err(io::Error::new(io::ErrorKind::TimedOut, "Youtube-dl timed out").into());
    };

    if !status.success() {
        event!(Level::ERROR, "Child process exited with error status: {status}");
        return Err(io::Error::other(format!("Youtube-dl exited with status `{status}`")).into());
    }

    let value: Value = serde_json::from_slice(&stdout)?;
    if let Some("playlist") = value.get("_type").and_then(Value::as_str) {
        let entries = value
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| serde_json::Error::custom("Missing or invalid playlist entries"))?;

        let videos = entries
            .iter()
            .map(|entry| serde_json::from_value(entry.clone()))
            .collect::<Result<Vec<Video>, _>>()?;

        Ok(VideosInYT::new(videos))
    } else {
        let video: Video = serde_json::from_value(value)?;
        Ok(VideosInYT::new(vec![video]))
    }
}
