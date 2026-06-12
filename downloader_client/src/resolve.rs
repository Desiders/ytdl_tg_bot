use proto::downloader::{music_resolver_client::MusicResolverClient, ResolveSourceRequest, ResolveSourceResponse};
use tonic::Code;
use tracing::{info, instrument};
use url::Url;

use crate::{authenticated_request, with_node_failover, NodeAttemptErrorKind, NodeRouter, ResolveSourceErrorKind};

/// DRM music platforms whose links must be resolved to a DRM-free source before yt-dlp can download.
#[must_use]
pub fn is_drm_platform(domain: Option<&str>) -> bool {
    let Some(domain) = domain else {
        return false;
    };
    let domain = domain.strip_prefix("www.").unwrap_or(domain);
    domain.ends_with("spotify.com")
        || domain.ends_with("spotify.link")
        || domain.ends_with("music.apple.com")
        || domain.ends_with("tidal.com")
        || domain.starts_with("music.amazon.")
        || domain.ends_with("deezer.com")
        || domain.ends_with("deezer.page.link")
}

/// Resolves a DRM-platform link to an equivalent DRM-free, downloadable URL. Returns `Ok(None)` for
/// non-DRM links so callers can pass the original URL straight through unchanged.
#[instrument(skip_all, fields(url = %url))]
pub async fn resolve_to_drm_free(router: &NodeRouter, url: &Url) -> Result<Option<Url>, ResolveSourceErrorKind> {
    if !is_drm_platform(url.domain()) {
        return Ok(None);
    }

    let response = resolve_drm_free_source(
        router,
        url.domain(),
        ResolveSourceRequest {
            url: url.as_str().to_owned(),
            country: None,
        },
    )
    .await?;

    let resolved = Url::parse(&response.download_url)?;
    info!(resolved = %resolved, platform = %response.platform, "Resolved DRM-free source");
    Ok(Some(resolved))
}

/// Resolves a DRM-platform track link to a DRM-free downloadable source via a routed node.
///
/// # Errors
///
/// Returns an error if no node is available, authentication metadata cannot be
/// built, or the RPC fails.
pub async fn resolve_drm_free_source(
    router: &NodeRouter,
    domain: Option<&str>,
    request: ResolveSourceRequest,
) -> Result<ResolveSourceResponse, ResolveSourceErrorKind> {
    with_node_failover(
        router,
        domain,
        |node| {
            let request = request.clone();
            async move {
                let mut client = MusicResolverClient::new(node.channel.clone());
                let response = client.resolve_drm_free_source(authenticated_request(request, &node.token)?).await?;
                Ok::<_, ResolveSourceErrorKind>(response.into_inner())
            }
        },
        classify_resolve_source_error,
    )
    .await
    .map_err(ResolveSourceErrorKind::from)
}

fn classify_resolve_source_error(err: &ResolveSourceErrorKind) -> NodeAttemptErrorKind {
    match err {
        ResolveSourceErrorKind::Rpc(status) if status.code() == Code::ResourceExhausted => NodeAttemptErrorKind::ResourceExhausted,
        ResolveSourceErrorKind::Rpc(status) if status.code() == Code::Unavailable => NodeAttemptErrorKind::Unavailable,
        ResolveSourceErrorKind::Rpc(status) if status.code() == Code::Unauthenticated => NodeAttemptErrorKind::Unauthenticated,
        _ => NodeAttemptErrorKind::Fatal,
    }
}

#[cfg(test)]
mod tests {
    use super::is_drm_platform;

    #[test]
    fn detects_drm_platforms() {
        for domain in [
            "open.spotify.com",
            "spotify.com",
            "spotify.link",
            "music.apple.com",
            "tidal.com",
            "listen.tidal.com",
            "music.amazon.com",
            "music.amazon.co.uk",
            "deezer.com",
            "www.deezer.com",
        ] {
            assert!(is_drm_platform(Some(domain)), "{domain} should be DRM");
        }
    }

    #[test]
    fn ignores_non_drm_platforms() {
        for domain in [
            "youtube.com",
            "youtu.be",
            "soundcloud.com",
            "music.youtube.com",
            "bandcamp.com",
            "example.com",
        ] {
            assert!(!is_drm_platform(Some(domain)), "{domain} should not be DRM");
        }
        assert!(!is_drm_platform(None));
    }
}
