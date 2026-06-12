use std::sync::Arc;

use proto::downloader::{music_resolver_server::MusicResolver, ResolveSourceRequest, ResolveSourceResponse};
use tonic::{Request, Response, Status};
use tracing::error;

use crate::services::odesli::{OdesliResolver, ResolveErrorKind};

pub struct MusicResolverService {
    pub resolver: Arc<OdesliResolver>,
}

#[tonic::async_trait]
impl MusicResolver for MusicResolverService {
    async fn resolve_drm_free_source(&self, request: Request<ResolveSourceRequest>) -> Result<Response<ResolveSourceResponse>, Status> {
        let request = request.into_inner();
        let url = request.url;
        let country = request.country;

        let resolved = self.resolver.resolve(&url, country.as_deref()).await.map_err(|err| {
            error!(url = %url, %err, "Resolve DRM-free source failed");
            resolve_error_status(err)
        })?;

        Ok(Response::new(ResolveSourceResponse {
            download_url: resolved.download_url,
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
        ResolveErrorKind::InvalidUrl(_) | ResolveErrorKind::Unsupported => Status::invalid_argument(err.to_string()),
        ResolveErrorKind::NoSource => Status::not_found(err.to_string()),
        ResolveErrorKind::RateLimited => Status::resource_exhausted(err.to_string()),
        ResolveErrorKind::Gone | ResolveErrorKind::Disabled => Status::failed_precondition(err.to_string()),
        ResolveErrorKind::Http(_) => Status::unavailable(err.to_string()),
        ResolveErrorKind::Decode(_) => Status::internal(err.to_string()),
    }
}
