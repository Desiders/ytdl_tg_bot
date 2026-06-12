use std::{collections::HashMap, time::Duration};

use reqwest::StatusCode;
use serde::Deserialize;
use tracing::{info, warn};
use url::Url;

const REQUEST_TIMEOUT_SECS: u64 = 15;
const DEFAULT_COUNTRY: &str = "US";
const USER_AGENT: &str = concat!("ytdl-tg-bot/", env!("CARGO_PKG_VERSION"));

/// DRM-free, yt-dlp-downloadable platforms in descending preference.
const PLATFORM_PRIORITY: [&str; 4] = ["youtubeMusic", "youtube", "soundcloud", "bandcamp"];

#[derive(Debug, thiserror::Error)]
pub enum ResolveErrorKind {
    #[error("Invalid url: {0}")]
    InvalidUrl(String),
    #[error("Unsupported or unrecognized music link")]
    Unsupported,
    #[error("No DRM-free source available for this track")]
    NoSource,
    #[error("Odesli rate limit exceeded; try again later")]
    RateLimited,
    #[error("Odesli API is no longer available (HTTP 410); the v1-alpha.1 API was retired")]
    Gone,
    #[error("Odesli request failed: {0}")]
    Http(String),
    #[error("Failed to decode Odesli response: {0}")]
    Decode(String),
    #[error("Music resolver is disabled")]
    Disabled,
}

/// The chosen DRM-free source plus the track metadata pulled from Odesli.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub download_url: String,
    pub platform: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub thumbnail_url: Option<String>,
    pub page_url: String,
}

pub struct OdesliResolver {
    client: reqwest::Client,
    base_url: Box<str>,
    api_key: Option<Box<str>>,
    enabled: bool,
}

impl OdesliResolver {
    #[must_use]
    pub fn new(enabled: bool, base_url: Box<str>, api_key: Option<Box<str>>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .user_agent(USER_AGENT)
            .build()
            .expect("Failed to build Odesli HTTP client");
        Self {
            client,
            base_url,
            api_key,
            enabled,
        }
    }

    /// Resolves `raw_url` to a DRM-free downloadable source.
    pub async fn resolve(&self, raw_url: &str, country: Option<&str>) -> Result<Resolved, ResolveErrorKind> {
        if !self.enabled {
            return Err(ResolveErrorKind::Disabled);
        }
        let url = Url::parse(raw_url.trim()).map_err(|err| ResolveErrorKind::InvalidUrl(err.to_string()))?;
        let country = normalize_country(country);

        let response = self.fetch(url.as_str(), &country).await?;
        let resolved = pick_source(&response)?;
        info!(
            url = %url,
            country = %country,
            platform = %resolved.platform,
            "Resolved DRM-free source"
        );
        Ok(resolved)
    }

    async fn fetch(&self, url: &str, country: &str) -> Result<OdesliResponse, ResolveErrorKind> {
        let response = self
            .send(url, country)
            .await
            .map_err(|err| ResolveErrorKind::Http(err.to_string()))?;
        let status = response.status();

        if status == StatusCode::TOO_MANY_REQUESTS {
            warn!(url, "Odesli returned 429 (rate limited)");
            return Err(ResolveErrorKind::RateLimited);
        }
        // The deprecated v1-alpha.1 API returns 410 once retired (after 2026-07-31).
        if status == StatusCode::GONE {
            warn!(url, "Odesli API returned 410 Gone; v1-alpha.1 has been retired");
            return Err(ResolveErrorKind::Gone);
        }
        // Odesli answers an unparseable/unrecognized link with 4xx rather than an empty body.
        if status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND {
            return Err(ResolveErrorKind::Unsupported);
        }
        if !status.is_success() {
            return Err(ResolveErrorKind::Http(format!("Odesli returned status {status}")));
        }
        response
            .json::<OdesliResponse>()
            .await
            .map_err(|err| ResolveErrorKind::Decode(err.to_string()))
    }

    async fn send(&self, url: &str, country: &str) -> Result<reqwest::Response, reqwest::Error> {
        let mut request = self
            .client
            .get(format!("{}/links", self.base_url))
            .query(&[("url", url), ("userCountry", country)]);
        if let Some(key) = &self.api_key {
            request = request.query(&[("key", key.as_ref())]);
        }
        request.send().await
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OdesliResponse {
    #[serde(default)]
    entity_unique_id: Option<String>,
    #[serde(default)]
    page_url: Option<String>,
    #[serde(default)]
    links_by_platform: HashMap<String, PlatformLink>,
    #[serde(default)]
    entities_by_unique_id: HashMap<String, Entity>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlatformLink {
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entity {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    artist_name: Option<String>,
    #[serde(default)]
    thumbnail_url: Option<String>,
}

/// Picks the highest-priority DRM-free source and attaches the track metadata from the
/// top-level `entityUniqueId` entity. Pure so the selection rules can be unit-tested.
fn pick_source(response: &OdesliResponse) -> Result<Resolved, ResolveErrorKind> {
    let (platform, link) = PLATFORM_PRIORITY
        .iter()
        .find_map(|&platform| response.links_by_platform.get(platform).map(|link| (platform, link)))
        .ok_or(ResolveErrorKind::NoSource)?;

    let entity = response
        .entity_unique_id
        .as_ref()
        .and_then(|id| response.entities_by_unique_id.get(id));

    Ok(Resolved {
        download_url: link.url.clone(),
        platform: platform.to_owned(),
        title: entity.and_then(|entity| entity.title.clone()),
        artist: entity.and_then(|entity| entity.artist_name.clone()),
        thumbnail_url: entity.and_then(|entity| entity.thumbnail_url.clone()),
        page_url: response.page_url.clone().unwrap_or_default(),
    })
}

fn normalize_country(country: Option<&str>) -> String {
    match country.map(str::trim).filter(|country| !country.is_empty()) {
        Some(country) => country.to_uppercase(),
        None => DEFAULT_COUNTRY.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_country, pick_source, OdesliResponse, PlatformLink, ResolveErrorKind};

    fn sample() -> OdesliResponse {
        serde_json::from_str(
            r#"{
                "entityUniqueId": "ITUNES_SONG::123",
                "pageUrl": "https://song.link/x",
                "linksByPlatform": {
                    "spotify": { "url": "https://open.spotify.com/track/abc" },
                    "youtube": { "url": "https://youtube.com/watch?v=abc" },
                    "youtubeMusic": { "url": "https://music.youtube.com/watch?v=abc" }
                },
                "entitiesByUniqueId": {
                    "ITUNES_SONG::123": { "title": "Song", "artistName": "Artist", "thumbnailUrl": "https://img/x.jpg" }
                }
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn picks_youtube_music_first() {
        let resolved = pick_source(&sample()).unwrap();
        assert_eq!(resolved.platform, "youtubeMusic");
        assert_eq!(resolved.download_url, "https://music.youtube.com/watch?v=abc");
        assert_eq!(resolved.title.as_deref(), Some("Song"));
        assert_eq!(resolved.artist.as_deref(), Some("Artist"));
        assert_eq!(resolved.thumbnail_url.as_deref(), Some("https://img/x.jpg"));
        assert_eq!(resolved.page_url, "https://song.link/x");
    }

    #[test]
    fn falls_back_in_priority_order() {
        let mut response = sample();
        response.links_by_platform.remove("youtubeMusic");
        response.links_by_platform.remove("youtube");
        response.links_by_platform.insert(
            "soundcloud".into(),
            PlatformLink {
                url: "https://soundcloud.com/x".into(),
            },
        );

        assert_eq!(pick_source(&response).unwrap().platform, "soundcloud");
    }

    #[test]
    fn errors_when_only_drm_sources() {
        let mut response = sample();
        response.links_by_platform.clear();
        response.links_by_platform.insert(
            "spotify".into(),
            PlatformLink {
                url: "https://open.spotify.com/track/abc".into(),
            },
        );

        assert!(matches!(pick_source(&response), Err(ResolveErrorKind::NoSource)));
    }

    #[test]
    fn country_defaults_and_normalizes() {
        assert_eq!(normalize_country(None), "US");
        assert_eq!(normalize_country(Some("")), "US");
        assert_eq!(normalize_country(Some(" gb ")), "GB");
    }
}
