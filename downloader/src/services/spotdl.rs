use std::{
    io,
    process::{Output, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::time;
use tracing::{info, trace, warn};
use url::Url;

use crate::{config::SpotdlConfig, utils::process_exit_error};

const RESOLVE_TIMEOUT_SECS: u64 = 300;
const PLATFORM: &str = "youtube";

#[derive(Debug, thiserror::Error)]
pub enum ResolveErrorKind {
    #[error("Invalid url: {0}")]
    InvalidUrl(String),
    #[error("No DRM-free source available for this track")]
    NoSource,
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Spotdl timed out")]
    Timeout,
    #[error("Music resolver is disabled")]
    Disabled,
}

/// The chosen DRM-free source.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub download_urls: Vec<String>,
    pub platform: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub thumbnail_url: Option<String>,
    pub page_url: String,
}

pub struct SpotdlResolver {
    cfg: Arc<SpotdlConfig>,
}

impl SpotdlResolver {
    #[must_use]
    pub const fn new(cfg: Arc<SpotdlConfig>) -> Self {
        Self { cfg }
    }

    /// Resolves `raw_url` to a DRM-free downloadable source via `spotdl url`.
    pub async fn resolve(&self, raw_url: &str) -> Result<Resolved, ResolveErrorKind> {
        if !self.cfg.enabled {
            return Err(ResolveErrorKind::Disabled);
        }
        let url = Url::parse(raw_url.trim()).map_err(|err| ResolveErrorKind::InvalidUrl(err.to_string()))?;

        let (program, base_args) = self.cfg.command_parts();
        trace!(program, ?base_args, url = %url, "Spotdl args");
        info!(url = %url, "Resolving DRM-free source");
        let started_at = Instant::now();

        let child = tokio::process::Command::new(program)
            .args(base_args)
            .arg("url")
            .arg(url.as_str())
            .args(["--threads", "1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let Output { status, stdout, stderr } = time::timeout(Duration::from_secs(RESOLVE_TIMEOUT_SECS), child.wait_with_output())
            .await
            .map_err(|_| ResolveErrorKind::Timeout)??;

        let stderr = String::from_utf8_lossy(&stderr);
        let stdout = String::from_utf8_lossy(&stdout);
        if !status.success() {
            let message = if stderr.trim().is_empty() { &stdout } else { &stderr };
            return Err(process_exit_error("Spotdl", status, message.trim()).into());
        }
        if !stderr.is_empty() {
            warn!(%stderr);
        }
        let download_urls = parse_urls(&stdout);
        if download_urls.is_empty() {
            return Err(ResolveErrorKind::NoSource);
        }

        info!(
            url = %url,
            urls_count = download_urls.len(),
            elapsed_ms = started_at.elapsed().as_millis(),
            "Resolved DRM-free source"
        );
        Ok(Resolved {
            download_urls,
            platform: PLATFORM.to_owned(),
            title: None,
            artist: None,
            thumbnail_url: None,
            page_url: String::new(),
        })
    }
}

/// `spotdl url` prints matched URLs to stdout, one per line (several for an album/playlist).
/// Ignores progress/noise lines.
fn parse_urls(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| Url::parse(line).is_ok_and(|url| matches!(url.scheme(), "http" | "https")))
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_urls;

    #[test]
    fn parses_single_url() {
        assert_eq!(
            parse_urls("https://music.youtube.com/watch?v=abc\n"),
            vec!["https://music.youtube.com/watch?v=abc"]
        );
    }

    #[test]
    fn parses_multiple_urls_in_order() {
        let stdout = "https://music.youtube.com/watch?v=a\nhttps://music.youtube.com/watch?v=b\n";
        assert_eq!(
            parse_urls(stdout),
            vec!["https://music.youtube.com/watch?v=a", "https://music.youtube.com/watch?v=b"]
        );
    }

    #[test]
    fn skips_noise_lines() {
        let stdout = "Processing query...\nhttps://music.youtube.com/watch?v=abc\n";
        assert_eq!(parse_urls(stdout), vec!["https://music.youtube.com/watch?v=abc"]);
    }

    #[test]
    fn no_output_is_empty() {
        assert!(parse_urls("").is_empty());
        assert!(parse_urls("No results found\n").is_empty());
    }
}
