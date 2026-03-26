use anyhow::Result;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use tonic::transport::Channel;
use ytdl_tg_bot_proto::downloader::{node_capabilities_client::NodeCapabilitiesClient, Empty};

use super::authenticated_request;

pub struct NodeHandle {
    pub address: Box<str>,
    pub token: Box<str>,
    pub max_concurrent: u32,
    pub channel: Channel,

    local_active_downloads: AtomicU32,
    remote_active_downloads: AtomicU32,
}

impl NodeHandle {
    pub(super) fn new(address: Box<str>, token: Box<str>, max_concurrent: u32, channel: Channel) -> Self {
        Self {
            address,
            token,
            max_concurrent,
            channel,
            local_active_downloads: AtomicU32::new(0),
            remote_active_downloads: AtomicU32::new(0),
        }
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

    pub fn update_remote_active_downloads(&self, active_downloads: u32) {
        self.remote_active_downloads.store(active_downloads, Relaxed);
    }

    pub async fn fetch_supported_domains(&self) -> Result<Vec<String>> {
        let mut client = NodeCapabilitiesClient::new(self.channel.clone());
        let response = client.get_supported_domains(authenticated_request(Empty {}, &self.token)?).await?;
        Ok(response.into_inner().domains_with_cookies)
    }

    #[must_use]
    pub fn estimated_active_downloads(&self) -> u32 {
        self.local_active_downloads
            .load(Relaxed)
            .max(self.remote_active_downloads.load(Relaxed))
    }

    #[must_use]
    pub fn has_capacity(&self) -> bool {
        self.estimated_active_downloads() < self.max_concurrent
    }
}
