use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use tonic::transport::Channel;
use ytdl_tg_bot_proto::downloader::{node_capabilities_client::NodeCapabilitiesClient, Empty};

use crate::authenticated_request;

#[derive(Debug, thiserror::Error)]
pub enum NodeHandleError {
    #[error(transparent)]
    Metadata(#[from] tonic::metadata::errors::InvalidMetadataValue),
    #[error(transparent)]
    Rpc(#[from] tonic::Status),
}

pub struct NodeHandle {
    pub name: Box<str>,
    pub address: Box<str>,
    pub token: Box<str>,
    pub channel: Channel,

    max_concurrent: AtomicU32,
    local_active_downloads: AtomicU32,
    remote_active_downloads: AtomicU32,
    available: AtomicBool,
}

impl NodeHandle {
    pub(crate) fn new(name: Box<str>, address: Box<str>, token: Box<str>, channel: Channel) -> Self {
        Self {
            name,
            address,
            token,
            channel,
            max_concurrent: AtomicU32::new(1),
            local_active_downloads: AtomicU32::new(0),
            remote_active_downloads: AtomicU32::new(0),
            available: AtomicBool::new(false),
        }
    }

    #[must_use]
    pub fn max_concurrent(&self) -> u32 {
        self.max_concurrent.load(Relaxed)
    }

    #[must_use]
    pub fn estimated_active_downloads(&self) -> u32 {
        self.local_active_downloads
            .load(Relaxed)
            .max(self.remote_active_downloads.load(Relaxed))
    }

    #[must_use]
    pub fn has_capacity(&self) -> bool {
        self.is_available() && self.estimated_active_downloads() < self.max_concurrent()
    }

    pub fn reserve_download_slot(&self) {
        self.local_active_downloads.fetch_add(1, Relaxed);
    }

    pub fn release_download_slot(&self) {
        let mut current = self.local_active_downloads.load(Relaxed);
        loop {
            let next = current.saturating_sub(1);
            match self.local_active_downloads.compare_exchange_weak(current, next, Relaxed, Relaxed) {
                Ok(_) => return,
                Err(actual) => current = actual,
            }
        }
    }

    pub fn update_remote_status(&self, active_downloads: u32, max_concurrent: u32) {
        self.remote_active_downloads.store(active_downloads, Relaxed);
        self.max_concurrent.store(max_concurrent, Relaxed);
        self.available.store(true, Relaxed);
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.available.load(Relaxed)
    }

    pub fn mark_unavailable(&self) {
        self.available.store(false, Relaxed);
    }

    pub async fn fetch_supported_domains(&self) -> Result<Vec<String>, NodeHandleError> {
        let mut client = NodeCapabilitiesClient::new(self.channel.clone());
        let response = client.get_supported_domains(authenticated_request(Empty {}, &self.token)?).await?;
        Ok(response.into_inner().domains_with_cookies)
    }

    pub async fn fetch_status(&self) -> Result<(u32, u32), NodeHandleError> {
        let mut client = NodeCapabilitiesClient::new(self.channel.clone());
        let response = client.get_status(authenticated_request(Empty {}, &self.token)?).await?;
        let status = response.into_inner();
        Ok((status.active_downloads, status.max_concurrent))
    }
}
