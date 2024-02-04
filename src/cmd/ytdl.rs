use crate::models::VideosInYT;

use futures_util::StreamExt as _;
use nix::{
    libc::{self, _exit, O_WRONLY, STDERR_FILENO, STDOUT_FILENO},
    unistd::{close, dup2, execv, fork, write, ForkResult, Pid},
};
use serde_json::{json, Value};
use std::{
    ffi::CString,
    io,
    os::fd::RawFd,
    path::Path,
    process::{Output, Stdio},
};
use tokio::process::Command;
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
fn download_to_pipe(_fd: RawFd, _executable_path: impl AsRef<str>, _dir_path: impl AsRef<Path>, _args: &[&str]) -> Result<Pid, io::Error> {
    unimplemented!("This function is only implemented for Unix systems");
}

/// Download video or audio stream to a pipe.
/// This function forks a child process and executes `yt-dl` in it.
/// The child process redirects its stdout to the pipe.
/// # Warning
/// The child process will `close` the fd, so you should not use it after calling this function.
/// # Errors
/// Returns [`io::Error`] if the `fork` fails.
/// # Returns
/// Returns the PID of the child process.
#[cfg(target_family = "unix")]
#[instrument(skip_all, fields(fd = %fd))]
fn download_to_pipe(fd: RawFd, executable_path: impl AsRef<str>, args: &[&str]) -> Result<Pid, io::Error> {
    event!(Level::TRACE, "Starting youtube-dl");

    let child = match unsafe { fork() } {
        Ok(ForkResult::Child) => unsafe {
            // Redirect `stderr` to `/dev/null`, because `ytdl` don't allow to disable logging level.
            let _ = dup2(libc::open("/dev/null\0".as_ptr().cast(), O_WRONLY), STDERR_FILENO);

            // Redirect `stdout` to the pipe.
            // Be aware and don't use `dup3` with `O_CLOEXEC` flag here.
            if let Err(errno) = dup2(fd, STDOUT_FILENO) {
                let _ = write(STDERR_FILENO, b"Error redirecting child process stdout to video write pipe");

                _exit(errno as i32);
            }

            match execv(
                &CString::new(executable_path.as_ref()).unwrap(),
                &args.iter().map(|arg| CString::new(*arg).unwrap()).collect::<Vec<_>>(),
            ) {
                Ok(_) => {
                    if let Err(errno) = close(fd) {
                        let _ = write(STDERR_FILENO, b"Error closing video write pipe");

                        _exit(errno as i32);
                    }

                    _exit(0);
                }
                Err(errno) => {
                    if let Err(errno) = close(fd) {
                        let _ = write(STDERR_FILENO, b"Error closing video write pipe");

                        _exit(errno as i32);
                    }

                    let _ = write(STDERR_FILENO, b"Error executing youtube-dl");

                    _exit(errno as i32);
                }
            }
        },
        Ok(ForkResult::Parent { child }) => child,
        Err(errno) => {
            event!(Level::ERROR, "Error forking process");

            return Err(errno.into());
        }
    };

    event!(Level::TRACE, %child, "Parent process spawned child process");

    Ok(child)
}

#[cfg(not(target_family = "unix"))]
pub fn download_video_to_pipe(
    _fd: RawFd,
    _executable_path: impl AsRef<str>,
    _id_or_url: impl AsRef<str>,
    _format: impl AsRef<str>,
) -> Result<Pid, io::Error> {
    unimplemented!("This function is only implemented for Unix systems");
}

/// Download video stream to a pipe.
/// This function forks a child process and executes `yt-dl` in it.
/// The child process redirects its stdout to the pipe.
/// # Warning
/// The child process will `close` the fd, so you should not use it after calling this function.
/// # Errors
/// Returns [`io::Error`] if the `fork` fails.
/// # Returns
/// Returns the PID of the child process.
#[cfg(target_family = "unix")]
pub fn download_video_to_pipe(
    fd: RawFd,
    executable_path: impl AsRef<str>,
    id_or_url: impl AsRef<str>,
    format: impl AsRef<str>,
) -> Result<Pid, io::Error> {
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
    _fd: RawFd,
    _executable_path: impl AsRef<str>,
    _id_or_url: impl AsRef<str>,
    _format: impl AsRef<str>,
) -> Result<Pid, io::Error> {
    unimplemented!("This function is only implemented for Unix systems");
}

/// Download audio stream to a pipe.
/// This function forks a child process and executes `yt-dl` in it.
/// The child process redirects its stdout to the pipe.
/// # Warning
/// The child process will `close` the fd, so you should not use it after calling this function.
/// # Errors
/// Returns [`io::Error`] if the `fork` fails.
/// # Returns
/// Returns the PID of the child process.
#[cfg(target_family = "unix")]
pub fn download_audio_stream_to_pipe(
    fd: RawFd,
    executable_path: impl AsRef<str>,
    id_or_url: impl AsRef<str>,
    format: impl AsRef<str>,
) -> Result<Pid, io::Error> {
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

    let Output { status, .. } = Command::new(executable_path.as_ref())
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

    let mut child = Command::new(executable_path.as_ref())
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
