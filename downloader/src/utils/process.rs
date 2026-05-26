use std::{io, process::ExitStatus};

/// Build an `io::Error` describing a subprocess that exited unsuccessfully.
/// Shared by the yt-dlp and gallery-dl wrappers to keep the messages consistent.
pub fn process_exit_error(name: &str, status: ExitStatus, stderr: &str) -> io::Error {
    match status.code() {
        Some(code) => io::Error::other(format!("{name} exited with code {code} and message: {stderr}")),
        None => io::Error::other(format!("{name} exited with and message: {stderr}")),
    }
}
