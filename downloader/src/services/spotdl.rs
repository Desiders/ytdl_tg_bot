use std::{
    io,
    process::{Output, Stdio},
    sync::Arc,
    time::Duration,
};

use tokio::time;
use tracing::{info, trace, warn};
use url::Url;

use crate::{config::SpotdlConfig, utils::process_exit_error};

const RESOLVE_TIMEOUT_SECS: u64 = 60;
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
    pub download_url: String,
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

        let child = tokio::process::Command::new(program)
            .args(base_args)
            .arg("url")
            .arg(url.as_str())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let Output { status, stdout, stderr } = time::timeout(Duration::from_secs(RESOLVE_TIMEOUT_SECS), child.wait_with_output())
            .await
            .map_err(|_| ResolveErrorKind::Timeout)??;

        let stderr = String::from_utf8_lossy(&stderr);
        if !status.success() {
            return Err(process_exit_error("Spotdl", status, &stderr).into());
        }
        if !stderr.is_empty() {
            warn!(%stderr);
        }

        let stdout = String::from_utf8_lossy(&stdout);
        let download_url = parse_first_url(&stdout).ok_or(ResolveErrorKind::NoSource)?;

        info!(url = %url, download_url = %download_url, "Resolved DRM-free source");
        Ok(Resolved {
            download_url,
            platform: PLATFORM.to_owned(),
            title: None,
            artist: None,
            thumbnail_url: None,
            page_url: String::new(),
        })
    }
}

/// `spotdl url` prints matched URLs to stdout, one per line (several for an album/playlist).
/// Takes the first valid URL; ignores progress/noise lines.
fn parse_first_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| Url::parse(line).is_ok_and(|url| matches!(url.scheme(), "http" | "https")))
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::parse_first_url;

    #[test]
    fn parses_single_url() {
        assert_eq!(
            parse_first_url("https://music.youtube.com/watch?v=abc\n").as_deref(),
            Some("https://music.youtube.com/watch?v=abc")
        );
    }

    #[test]
    fn takes_first_of_multiple_urls() {
        let stdout = "https://music.youtube.com/watch?v=a\nhttps://music.youtube.com/watch?v=b\n";
        assert_eq!(parse_first_url(stdout).as_deref(), Some("https://music.youtube.com/watch?v=a"));
    }

    #[test]
    fn skips_noise_lines() {
        let stdout = "Processing query...\nhttps://music.youtube.com/watch?v=abc\n";
        assert_eq!(parse_first_url(stdout).as_deref(), Some("https://music.youtube.com/watch?v=abc"));
    }

    #[test]
    fn no_output_is_none() {
        assert_eq!(parse_first_url(""), None);
        assert_eq!(parse_first_url("No results found\n"), None);
    }
}
