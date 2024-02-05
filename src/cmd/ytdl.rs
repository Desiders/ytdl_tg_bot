use crate::models::VideosInYT;

use futures_util::StreamExt as _;
use serde_json::{json, Value};
use std::{
    io,
    os::fd::OwnedFd,
    path::Path,
    process::{Command, Output, Stdio},
};
use tokio::process::Command as TokioCommand;
use tokio_util::codec::{FramedRead, LinesCodec};
use tracing::{event, instrument, Level};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Line codec error: {0}")]
    Line(#[from] tokio_util::codec::LinesCodecError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(not(target_family = "unix"))]
fn download_to_pipe(
    _fd: OwnedFd,
    _executable_path: impl AsRef<str>,
    _dir_path: impl AsRef<Path>,
    _args: &[&str],
) -> Result<u32, io::Error> {
    unimplemented!("This function is only implemented for Unix systems");
}

/// Download video or audio stream to a pipe.
/// This function forks a child process and executes `yt-dl` in it.
/// The child process redirects its stdout to the pipe.
/// # Errors
/// Returns [`io::Error`] if the spawn child process fails.
/// # Returns
/// Returns the PID of the child process.
#[cfg(target_family = "unix")]
#[instrument(skip_all, fields(fd = ?fd))]
fn download_to_pipe(fd: OwnedFd, executable_path: impl AsRef<str>, args: &[&str]) -> Result<u32, io::Error> {
    event!(Level::TRACE, "Starting youtube-dl");

    Command::new(executable_path.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(fd))
        .stderr(Stdio::null())
        .spawn()
        .map(|child| child.id())
}

#[cfg(not(target_family = "unix"))]
pub fn download_video_to_pipe(
    _fd: OwnedFd,
    _executable_path: impl AsRef<str>,
    _id_or_url: impl AsRef<str>,
    _format: impl AsRef<str>,
) -> Result<Pid, io::Error> {
    unimplemented!("This function is only implemented for Unix systems");
}

/// Download video stream to a pipe.
/// This function forks a child process and executes `yt-dl` in it.
/// The child process redirects its stdout to the pipe.
/// # Errors
/// Returns [`io::Error`] if the spawn child process fails.
/// # Returns
/// Returns the PID of the child process.
#[cfg(target_family = "unix")]
pub fn download_video_to_pipe(
    fd: OwnedFd,
    executable_path: impl AsRef<str>,
    id_or_url: impl AsRef<str>,
    format: impl AsRef<str>,
) -> Result<u32, io::Error> {
    let args = [
        "--no-update",
        "--ignore-config",
        "--no-config-locations",
        "--abort-on-error",
        "--color",
        "never",
        "--no-match-filters",
        "--no-download-archive",
        "--socket-timeout",
        "5",
        "--concurrent-fragments",
        "4",
        "--resize-buffer",
        "--no-batch-file",
        "--output",
        "-",
        "--no-playlist",
        "--no-mtime",
        "--no-write-description",
        "--no-write-info-json",
        "--no-write-comments",
        "--no-cookies",
        "--no-cookies-from-browser",
        "--no-write-thumbnail",
        "--no-ignore-no-formats-error",
        "--no-simulate",
        "--no-progress",
        "--no-check-certificate",
        "--no-video-multistreams",
        "--no-check-formats",
        "--no-write-subs",
        "--no-write-auto-subs",
        "-f",
        format.as_ref(),
        id_or_url.as_ref(),
    ];

    download_to_pipe(fd, executable_path, &args)
}

#[cfg(not(target_family = "unix"))]
pub fn download_audio_stream_to_pipe(
    _fd: OwnedFd,
    _executable_path: impl AsRef<str>,
    _id_or_url: impl AsRef<str>,
    _format: impl AsRef<str>,
) -> Result<u32, io::Error> {
    unimplemented!("This function is only implemented for Unix systems");
}

/// Download audio stream to a pipe.
/// This function forks a child process and executes `yt-dl` in it.
/// The child process redirects its stdout to the pipe.
/// # Errorss
/// Returns [`io::Error`] if the spawn child process fails.
/// # Returns
/// Returns the PID of the child process.
#[cfg(target_family = "unix")]
pub fn download_audio_stream_to_pipe(
    fd: OwnedFd,
    executable_path: impl AsRef<str>,
    id_or_url: impl AsRef<str>,
    format: impl AsRef<str>,
) -> Result<u32, io::Error> {
    let args = [
        "--no-update",
        "--ignore-config",
        "--no-config-locations",
        "--abort-on-error",
        "--color",
        "never",
        "--no-match-filters",
        "--no-download-archive",
        "--socket-timeout",
        "5",
        "--concurrent-fragments",
        "4",
        "--resize-buffer",
        "--no-batch-file",
        "--output",
        "-",
        "--no-playlist",
        "--no-mtime",
        "--no-write-description",
        "--no-write-info-json",
        "--no-write-comments",
        "--no-cookies",
        "--no-cookies-from-browser",
        "--no-write-thumbnail",
        "--no-ignore-no-formats-error",
        "--no-simulate",
        "--no-progress",
        "--no-check-certificate",
        "--no-video-multistreams",
        "--no-check-formats",
        "--no-write-subs",
        "--no-write-auto-subs",
        "-f",
        format.as_ref(),
        id_or_url.as_ref(),
    ];

    download_to_pipe(fd, executable_path, &args)
}

pub async fn download_audio_to_path(
    executable_path: impl AsRef<str>,
    id_or_url: impl AsRef<str>,
    format: impl AsRef<str>,
    output_extension: impl AsRef<str>,
    output_dir_path: impl AsRef<Path>,
) -> Result<(), Error> {
    let output_dir_path = output_dir_path.as_ref().to_string_lossy();

    let args = [
        "--no-update",
        "--ignore-config",
        "--no-config-locations",
        "--abort-on-error",
        "--color",
        "never",
        "--no-match-filters",
        "--no-download-archive",
        "--socket-timeout",
        "5",
        "--concurrent-fragments",
        "4",
        "--resize-buffer",
        "--no-batch-file",
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
        "--write-all-thumbnail",
        "--no-mtime",
        "--no-write-description",
        "--no-write-info-json",
        "--no-write-comments",
        "--no-cookies",
        "--no-cookies-from-browser",
        "--quiet",
        "--no-ignore-no-formats-error",
        "--no-simulate",
        "--no-progress",
        "--no-check-certificate",
        "--no-video-multistreams",
        "--no-check-formats",
        "--no-write-subs",
        "--no-write-auto-subs",
        "-f",
        format.as_ref(),
        id_or_url.as_ref(),
    ];

    let Output { status, .. } = TokioCommand::new(executable_path.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .await?;

    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, format!("Youtube-dl exited with status `{status}`")).into());
    }

    Ok(())
}

pub async fn get_media_or_playlist_info(
    executable_path: impl AsRef<str>,
    id_or_url: impl AsRef<str>,
    allow_playlist: bool,
) -> Result<VideosInYT, Error> {
    let args = [
        "--no-update",
        "--ignore-config",
        "--no-config-locations",
        "--abort-on-error",
        "--color",
        "never",
        "--no-match-filters",
        "--socket-timeout",
        "5",
        "--resize-buffer",
        "--no-batch-file",
        "--output",
        "%(id)s.%(ext)s",
        if allow_playlist { "--yes-playlist" } else { "--no-playlist" },
        "--no-mtime",
        "--no-write-description",
        "--no-write-info-json",
        "--no-write-comments",
        "--no-cookies",
        "--no-cookies-from-browser",
        "--no-write-thumbnail",
        "--quiet",
        "--no-ignore-no-formats-error",
        "--skip-download",
        "--simulate",
        "--no-progress",
        "--no-check-certificate",
        "--no-video-multistreams",
        "--no-check-formats",
        "--no-write-subs",
        "--no-write-auto-subs",
        "-J",
        id_or_url.as_ref(),
    ];

    let mut child = TokioCommand::new(executable_path.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let mut videos = vec![];
    let mut stdout = FramedRead::new(child.stdout.take().unwrap(), LinesCodec::new());

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

    let status = child.wait().await?;

    if !status.success() {
        event!(Level::ERROR, "Child process exited with error status: {status}");

        return Err(io::Error::new(io::ErrorKind::Other, format!("Youtube-dl exited with status `{status}`")).into());
    }

    Ok(VideosInYT::new(videos))
}
