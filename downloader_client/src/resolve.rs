use proto::downloader::{music_resolver_client::MusicResolverClient, ResolveSourceRequest, ResolveSourceResponse};
use tonic::Code;
use tracing::{info, instrument};
use url::Url;

use crate::{authenticated_request, with_node_failover, NodeAttemptErrorKind, NodeRouter, ResolveSourceErrorKind};

/// Spotify links must be resolved to a DRM-free source before yt-dlp can download;
/// the spotdl resolver on the node does not accept other platforms.
#[must_use]
pub fn is_spotify_platform(domain: Option<&str>) -> bool {
    let Some(domain) = domain else {
        return false;
    };
    let domain = domain.strip_prefix("www.").unwrap_or(domain);
    domain.ends_with("spotify.com") || domain.ends_with("spotify.link")
}

/// Resolves a DRM-platform link to equivalent DRM-free, downloadable URLs (several for an
/// album/playlist). Returns `Ok(None)` for non-DRM links so callers can pass the original URL
/// straight through unchanged.
#[instrument(skip_all, fields(url = %url))]
pub async fn resolve_to_drm_free(router: &NodeRouter, url: &Url) -> Result<Option<Vec<Url>>, ResolveSourceErrorKind> {
    if !is_spotify_platform(url.domain()) {
        return Ok(None);
    }

    let response = resolve_drm_free_source(
        router,
        url.domain(),
        ResolveSourceRequest {
            url: url.as_str().to_owned(),
        },
    )
    .await?;

    let resolved = response
        .download_urls
        .iter()
        .map(|url| Url::parse(url))
        .collect::<Result<Vec<_>, _>>()?;
    info!(urls_count = resolved.len(), platform = %response.platform, "Resolved DRM-free source");
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
    use super::is_spotify_platform;

    #[test]
    fn detects_spotify_platforms() {
        for domain in ["open.spotify.com", "spotify.com", "www.spotify.com", "spotify.link"] {
            assert!(is_spotify_platform(Some(domain)), "{domain} should be Spotify");
        }
    }

    #[test]
    fn ignores_non_spotify_platforms() {
        for domain in [
            "youtube.com",
            "youtu.be",
            "soundcloud.com",
            "music.youtube.com",
            "bandcamp.com",
            "example.com",
        ] {
            assert!(!is_spotify_platform(Some(domain)), "{domain} should not be Spotify");
        }
        assert!(!is_spotify_platform(None));
    }
}
