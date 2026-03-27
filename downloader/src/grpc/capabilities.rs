use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};
use tonic::{Request, Response, Status};
use tracing::info;
use ytdl_tg_bot_proto::downloader::{node_capabilities_server::NodeCapabilities, Empty, NodeStatus, SupportedDomainsResponse};

use crate::entities::Cookies;

pub struct CapabilitiesService {
    pub cookies: Arc<Cookies>,
    pub active_downloads: Arc<AtomicU32>,
    pub max_concurrent: u32,
}

#[tonic::async_trait]
impl NodeCapabilities for CapabilitiesService {
    async fn get_status(&self, _request: Request<Empty>) -> Result<Response<NodeStatus>, Status> {
        let active_downloads = self.active_downloads.load(Ordering::Relaxed);
        info!(active_downloads, max_concurrent = self.max_concurrent, "Reported node status");

        Ok(Response::new(NodeStatus {
            active_downloads,
            max_concurrent: self.max_concurrent,
        }))
    }

    async fn get_supported_domains(&self, _request: Request<Empty>) -> Result<Response<SupportedDomainsResponse>, Status> {
        let domains_with_cookies: Vec<_> = self.cookies.get_hosts().into_iter().map(ToString::to_string).collect();
        info!(domain_count = domains_with_cookies.len(), domains = ?domains_with_cookies, "Reported supported cookie domains");

        Ok(Response::new(SupportedDomainsResponse { domains_with_cookies }))
    }
}
