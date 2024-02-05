use std::{
    io,
    os::fd::RawFd,
    path::Path,
    process::{Command, Stdio},
};
use tracing::{event, instrument, Level};

/// Merge the video and audio streams into a single file.
/// This function forks a child process and executes `ffmpeg` in it.
/// # Errors
/// Returns [`io::Error`] if the spawn child process fails.
/// # Returns
/// Returns the PID of the child process.
#[instrument(skip_all, fields(%video_fd, %audio_fd, output_path = %output_path.as_ref().as_os_str().to_string_lossy()))]
pub fn merge_streams(
    video_fd: RawFd,
    audio_fd: RawFd,
    extension: impl AsRef<str>,
    output_path: impl AsRef<Path>,
) -> Result<u32, io::Error> {
    event!(Level::TRACE, "Starting ffmpeg");

    Command::new("/usr/bin/ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            &format!("pipe:{video_fd}"),
            "-i",
            &format!("pipe:{audio_fd}"),
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "copy",
            "-c:a",
            "copy",
            "-preset",
            "ultrafast",
            "-f",
            extension.as_ref(),
            output_path.as_ref().to_string_lossy().as_ref(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map(|child| child.id())
}
