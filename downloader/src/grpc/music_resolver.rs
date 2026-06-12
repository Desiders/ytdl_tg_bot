use std::sync::Arc;

use proto::downloader::{music_resolver_server::MusicResolver, ResolveSourceRequest, ResolveSourceResponse};
use tonic::{Request, Response, Status};
use tracing::error;

use crate::services::spotdl::{ResolveErrorKind, SpotdlResolver};

pub struct MusicResolverService {
    pub resolver: Arc<SpotdlResolver>,
}

#[tonic::async_trait]
impl MusicResolver for MusicResolverService {
    async fn resolve_drm_free_source(&self, request: Request<ResolveSourceRequest>) -> Result<Response<ResolveSourceResponse>, Status> {
        let request = request.into_inner();
        let url = request.url;

        let resolved = self.resolver.resolve(&url).await.map_err(|err| {
            error!(url = %url, %err, "Resolve DRM-free source failed");
            resolve_error_status(err)
        })?;

        Ok(Response::new(ResolveSourceResponse {
            download_urls: resolved.download_urls,
            platform: resolved.platform,
            title: resolved.title,
            artist: resolved.artist,
            thumbnail_url: resolved.thumbnail_url,
            page_url: resolved.page_url,
        }))
    }
}

fn resolve_error_status(err: ResolveErrorKind) -> Status {
    match err {
        ResolveErrorKind::InvalidUrl(_) => Status::invalid_argument(err.to_string()),
        ResolveErrorKind::NoSource => Status::not_found(err.to_string()),
        ResolveErrorKind::Disabled => Status::failed_precondition(err.to_string()),
        ResolveErrorKind::Io(_) | ResolveErrorKind::Timeout => Status::internal(err.to_string()),
    }
}
