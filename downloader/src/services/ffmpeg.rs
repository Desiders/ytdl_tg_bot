use serde::Deserialize;
use std::{
    io,
    path::Path,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{process::Command, time};
use tracing::instrument;

#[derive(Debug, thiserror::Error)]
pub enum ConvertErrorKind {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProbedVideo {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<f32>,
}

#[derive(Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    #[serde(default)]
    format: FfprobeFormat,
}

#[derive(Deserialize, Default)]
struct FfprobeStream {
    width: Option<i64>,
    height: Option<i64>,
}

#[derive(Deserialize, Default)]
struct FfprobeFormat {
    duration: Option<String>,
}

#[instrument(skip_all)]
pub async fn probe_video(file_path: &Path, executable_path: &str, timeout: u64) -> ProbedVideo {
    let file_path = file_path.to_string_lossy();
    let args = [
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height:format=duration",
        "-of",
        "json",
        &file_path,
    ];
    let Ok(child) = Command::new(executable_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    else {
        return ProbedVideo::default();
    };
    match time::timeout(Duration::from_secs(timeout), child.wait_with_output()).await {
        Ok(Ok(output)) if output.status.success() => parse_probe(&output.stdout),
        _ => ProbedVideo::default(),
    }
}

fn parse_probe(stdout: &[u8]) -> ProbedVideo {
    let Ok(parsed) = serde_json::from_slice::<FfprobeOutput>(stdout) else {
        return ProbedVideo::default();
    };
    let stream = parsed.streams.into_iter().next().unwrap_or_default();
    ProbedVideo {
        width: stream.width,
        height: stream.height,
        duration: parsed.format.duration.and_then(|duration| duration.parse().ok()),
    }
}

#[instrument(skip_all)]
pub async fn download_and_convert(url: &str, output_file_path: &Path, executable_path: &str, timeout: u64) -> Result<(), ConvertErrorKind> {
    let output_file_path = output_file_path.to_string_lossy();
    let args = ["-y", "-hide_banner", "-loglevel", "error", "-i", url, &output_file_path];
    run_ffmpeg(&args, executable_path, timeout).await
}

#[instrument(skip_all)]
pub async fn remux_copy(url: &str, output_file_path: &Path, executable_path: &str, timeout: u64) -> Result<(), ConvertErrorKind> {
    let output_file_path = output_file_path.to_string_lossy();
    let args = [
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        url,
        "-c",
        "copy",
        "-movflags",
        "+faststart",
        &output_file_path,
    ];
    run_ffmpeg(&args, executable_path, timeout).await
}

#[instrument(skip_all)]
pub async fn extract_audio(url: &str, output_file_path: &Path, executable_path: &str, timeout: u64) -> Result<(), ConvertErrorKind> {
    let output_file_path = output_file_path.to_string_lossy();
    let copy_args = [
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        url,
        "-vn",
        "-c:a",
        "copy",
        "-movflags",
        "+faststart",
        &output_file_path,
    ];
    if run_ffmpeg(&copy_args, executable_path, timeout).await.is_ok() {
        return Ok(());
    }
    let aac_args = [
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        url,
        "-vn",
        "-c:a",
        "aac",
        "-movflags",
        "+faststart",
        &output_file_path,
    ];
    run_ffmpeg(&aac_args, executable_path, timeout).await
}

async fn run_ffmpeg(args: &[&str], executable_path: &str, timeout: u64) -> Result<(), ConvertErrorKind> {
    let child = Command::new(executable_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    match time::timeout(Duration::from_secs(timeout), child.wait_with_output()).await {
        Ok(Ok(Output { status, stderr, .. })) => {
            let stderr = String::from_utf8_lossy(&stderr);
            if status.success() {
                Ok(())
            } else {
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
