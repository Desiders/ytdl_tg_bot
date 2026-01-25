use std::{
    io,
    path::Path,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{process::Command, time};
use tracing::{error, instrument};

#[derive(Debug, thiserror::Error)]
pub enum ConvertErrorKind {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

#[instrument(skip_all)]
pub async fn download_and_convert(url: &str, output_file_path: &Path, executable_path: &str, timeout: u64) -> Result<(), ConvertErrorKind> {
    let output_file_path = output_file_path.to_string_lossy();

    let args = ["-y", "-hide_banner", "-loglevel", "error", "-i", url, &output_file_path];

    let child = Command::new(executable_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    match time::timeout(Duration::from_secs(timeout), child.wait_with_output()).await {
        Ok(Ok(Output { status, stderr, .. })) => {
            if status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&stderr);
                match status.code() {
                    Some(code) => Err(io::Error::other(format!("Ffmpeg exited with code {code} and message: {stderr}")).into()),
                    None => Err(io::Error::other(format!("Ffmpeg exited with and message: {stderr}")).into()),
                }
            }
        }
        Ok(Err(err)) => Err(err.into()),
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "Ffmpeg timed out").into()),
    }
}
