use std::{convert::Infallible, sync::Arc};
use tracing::{info, instrument};
pub use ytdl_tg_downloader_client::{
    download_media, get_media_info, DownloadErrorKind, DownloadEvent, DownloadSession, DownloaderServiceTarget, GetMediaInfoErrorKind,
    NodeRouter,
};

use crate::{entities::NodeStats, errors::ErrorKind, interactors::Interactor};

pub struct GetStats {
    pub node_router: Arc<NodeRouter>,
}

pub struct GetStatsInput {}

impl<'a> Interactor<GetStatsInput> for &'a GetStats {
    type Output = Vec<NodeStats>;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, GetStatsInput {}: GetStatsInput) -> Result<Self::Output, Self::Err> {
        let nodes = self.node_router.nodes();
        let mut nodes_stats = Vec::with_capacity(nodes.len());
        for node in nodes {
            let node_stats = NodeStats {
                name: node.name.clone(),
                active_downloads: node.estimated_active_downloads(),
                max_concurrent: node.max_concurrent(),
            };
            info!(?node_stats, "Got nodes stats");

            nodes_stats.push(node_stats);
        }
        Ok(nodes_stats)
    }
}
