use proto::downloader::{node_capabilities_server::NodeCapabilities, Empty, NodeStatus, SupportedDomainsResponse};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tonic::{Request, Response, Status};
use tracing::info;

use crate::{entities::Cookies, grpc::active_downloads};

pub struct CapabilitiesService {
    pub cookies: Arc<Cookies>,
    pub semaphore: Arc<Semaphore>,
    pub max_concurrent: u32,
}

#[tonic::async_trait]
impl NodeCapabilities for CapabilitiesService {
    async fn get_status(&self, _request: Request<Empty>) -> Result<Response<NodeStatus>, Status> {
        let active_downloads = active_downloads(&self.semaphore, self.max_concurrent);
        info!(active_downloads, max_concurrent = self.max_concurrent, "Reported node status");

        Ok(Response::new(NodeStatus {
            active_downloads,
            max_concurrent: self.max_concurrent,
        }))
    }

    async fn get_supported_domains(&self, _request: Request<Empty>) -> Result<Response<SupportedDomainsResponse>, Status> {
        let domains_with_cookies = self.cookies.get_domains();
        info!(domain_count = domains_with_cookies.len(), domains = ?domains_with_cookies, "Reported supported cookie domains");

        Ok(Response::new(SupportedDomainsResponse { domains_with_cookies }))
    }
}
