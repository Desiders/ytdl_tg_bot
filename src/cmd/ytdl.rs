use crate::models::{VideoInYT, VideosInYT};

use serde::de::Error as _;
use serde_json::{json, Value};
use std::{
    io::{self, Read},
    os::fd::OwnedFd,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tracing::{event, instrument, Level};
use wait_timeout::ChildExt as _;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Download video or audio stream to a pipe.
/// This function forks a child process and executes `yt-dl` in it.
/// The child process redirects its stdout to the pipe.
/// # Errors
/// Returns [`io::Error`] if the spawn child process fails.
/// # Returns
/// Returns the child process.
#[instrument(skip_all, fields(fd = ?fd))]
fn download_to_pipe(fd: OwnedFd, executable_path: impl AsRef<str>, args: &[&str]) -> Result<Child, io::Error> {
    event!(Level::TRACE, "Starting youtube-dl");

    Command::new(executable_path.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(fd))
        .stderr(Stdio::null())
        .spawn()
}

/// Download video stream to a pipe.
/// This function forks a child process and executes `yt-dl` in it.
/// The child process redirects its stdout to the pipe.
/// # Errors
/// Returns [`io::Error`] if the spawn child process fails.
/// # Returns
/// Returns the child process.
pub fn download_video_to_pipe(
    fd: OwnedFd,
    executable_path: impl AsRef<str>,
    id_or_url: impl AsRef<str>,
    format: impl AsRef<str>,
) -> Result<Child, io::Error> {
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

/// Download audio stream to a pipe.
/// This function forks a child process and executes `yt-dl` in it.
/// The child process redirects its stdout to the pipe.
/// # Errorss
/// Returns [`io::Error`] if the spawn child process fails.
/// # Returns
/// Returns the child process.
pub fn download_audio_stream_to_pipe(
    fd: OwnedFd,
    executable_path: impl AsRef<str>,
    id_or_url: impl AsRef<str>,
    format: impl AsRef<str>,
) -> Result<Child, io::Error> {
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

pub fn download_video_to_path(
    executable_path: impl AsRef<str>,
    id_or_url: impl AsRef<str>,
    format: impl AsRef<str>,
    output_dir_path: impl AsRef<Path>,
    timeout: u64,
) -> Result<(), io::Error> {
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

    let mut child = Command::new(executable_path.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()?;

    let Some(exit_code) = child.wait_timeout(Duration::from_secs(timeout))? else {
        event!(Level::ERROR, "Child process timed out");

        child.kill()?;

        return Err(io::Error::new(io::ErrorKind::TimedOut, "Youtube-dl timed out"));
    };

    if !exit_code.success() {
        event!(Level::ERROR, "Child process exited with error status: {exit_code}");

        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Youtube-dl exited with status `{exit_code}`"),
        ));
    }

    Ok(())
}

pub fn download_audio_to_path(
    executable_path: impl AsRef<str>,
    id_or_url: impl AsRef<str>,
    format: impl AsRef<str>,
    output_extension: impl AsRef<str>,
    output_dir_path: impl AsRef<Path>,
    timeout: u64,
) -> Result<(), io::Error> {
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

    let mut child = Command::new(executable_path.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()?;

    let Some(exit_code) = child.wait_timeout(Duration::from_secs(timeout))? else {
        event!(Level::ERROR, "Child process timed out");

        child.kill()?;

        return Err(io::Error::new(io::ErrorKind::TimedOut, "Youtube-dl timed out"));
    };

    if !exit_code.success() {
        event!(Level::ERROR, "Child process exited with error status: {exit_code}");

        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Youtube-dl exited with status `{exit_code}`"),
        ));
    }

    Ok(())
}

pub fn get_media_or_playlist_info(
    executable_path: impl AsRef<str>,
    id_or_url: impl AsRef<str>,
    allow_playlist: bool,
    timeout: u64,
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
