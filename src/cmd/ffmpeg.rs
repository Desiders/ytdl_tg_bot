use nix::{
    libc::{_exit, STDERR_FILENO},
    unistd::{close, execve, fork, write, ForkResult, Pid},
};
use std::{
    ffi::CString,
    io,
    os::{fd::RawFd, unix::prelude::OsStrExt as _},
    path::Path,
};
use tracing::{event, instrument, Level};

#[cfg(not(target_family = "unix"))]
pub fn merge_streams(
    _video_fd: RawFd,
    _audio_fd: RawFd,
    _extension: impl AsRef<str>,
    _output_path: impl AsRef<Path>,
) -> Result<Pid, io::Error> {
    unimplemented!("This function is only implemented for Unix systems");
}

/// Merge the video and audio streams into a single file.
/// This function forks a child process and executes `ffmpeg` in it.
/// # Warning
/// The child process will `close` the read ends of the pipes, so you should not use them after calling this function.
/// # Errors
/// Returns [`io::Error`] if the `fork` fails.
/// # Returns
/// Returns the PID of the child process.
#[cfg(target_family = "unix")]
#[instrument(skip_all, fields(%video_fd, %audio_fd, output_path = %output_path.as_ref().as_os_str().to_string_lossy()))]
pub fn merge_streams(
    video_fd: RawFd,
    audio_fd: RawFd,
    extension: impl AsRef<str>,
    output_path: impl AsRef<Path>,
) -> Result<Pid, io::Error> {
    event!(Level::TRACE, "Starting ffmpeg");

    let child = match unsafe { fork() } {
        Ok(ForkResult::Child) => unsafe {
            match execve(
                &CString::new("/usr/bin/ffmpeg").unwrap(),
                &[
                    CString::new("-y").unwrap(),
                    CString::new("-hide_banner").unwrap(),
                    CString::new("-loglevel").unwrap(),
                    CString::new("error").unwrap(),
                    CString::new("-i").unwrap(),
                    CString::new(format!("pipe:{video_fd}")).unwrap(),
                    CString::new("-i").unwrap(),
                    CString::new(format!("pipe:{audio_fd}")).unwrap(),
                    CString::new("-map").unwrap(),
                    CString::new("0:v").unwrap(),
                    CString::new("-map").unwrap(),
                    CString::new("1:a").unwrap(),
                    CString::new("-c:v").unwrap(),
                    CString::new("copy").unwrap(),
                    CString::new("-c:a").unwrap(),
                    CString::new("copy").unwrap(),
                    CString::new("-preset").unwrap(),
                    CString::new("ultrafast").unwrap(),
                    CString::new("-f").unwrap(),
                    CString::new(extension.as_ref()).unwrap(),
                    CString::new(output_path.as_ref().as_os_str().as_bytes()).unwrap(),
                ],
                &[CString::new("PATH=/usr/bin").unwrap(), CString::new("HOME=/tmp").unwrap()],
            ) {
                Ok(_) => {
                    _exit(0);
                }
                Err(errno) => {
                    if let Err(errno) = close(video_fd) {
                        let _ = write(STDERR_FILENO, b"Error closing video read pipe");

                        _exit(errno as i32);
                    }

                    if let Err(errno) = close(audio_fd) {
                        let _ = write(STDERR_FILENO, b"Error closing audio read pipe");

                        _exit(errno as i32);
                    }

                    let _ = write(STDERR_FILENO, b"Error executing ffmpeg");

                    _exit(errno as i32);
                }
            };
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
