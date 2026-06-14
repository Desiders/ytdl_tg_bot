pub use downloader_client::{
    download_media, get_media_info, recognize_song, resolve_to_drm_free, DownloadErrorKind, DownloadEvent, DownloadSession,
    DownloaderServiceTarget, GetMediaInfoErrorKind, NodeRouter, RecognizedSong, ResolveSourceErrorKind,
};
use std::{convert::Infallible, sync::Arc};
use tracing::{info, instrument};

use crate::{entities::NodeStats, errors::ErrorKind, interactors::Interactor};

pub struct GetStats {
    node_router: Arc<NodeRouter>,
}

impl GetStats {
    #[must_use]
    pub const fn new(node_router: Arc<NodeRouter>) -> Self {
        Self { node_router }
    }
}

pub struct GetStatsInput {}

impl Interactor<GetStatsInput> for &GetStats {
    type Output = Vec<NodeStats>;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, GetStatsInput {}: GetStatsInput) -> Result<Self::Output, Self::Err> {
        let nodes = self.node_router.nodes();
        let mut stats = Vec::with_capacity(nodes.len());
        for node in nodes {
            let node_stats = NodeStats {
                name: node.name.clone(),
                active_downloads: node.estimated_active_downloads(),
                max_concurrent: node.max_concurrent(),
            };
            info!(?node_stats, "Got nodes stats");

            stats.push(node_stats);
        }
        Ok(stats)
    }
}
