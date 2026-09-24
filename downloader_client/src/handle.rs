use proto::downloader::{node_capabilities_client::NodeCapabilitiesClient, Empty};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use tonic::transport::Channel;

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
    active_downloads: AtomicU32,
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
            active_downloads: AtomicU32::new(0),
            available: AtomicBool::new(false),
        }
    }

    #[must_use]
    pub fn max_concurrent(&self) -> u32 {
        self.max_concurrent.load(Relaxed)
    }

    #[must_use]
    pub fn active_downloads(&self) -> u32 {
        self.active_downloads.load(Relaxed)
    }

    pub fn update_remote_status(&self, active_downloads: u32, max_concurrent: u32) {
        self.active_downloads.store(active_downloads, Relaxed);
        self.max_concurrent.store(max_concurrent, Relaxed);
        self.available.store(true, Relaxed);
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.available.load(Relaxed)
    }

    pub fn mark_unavailable(&self) {
        self.available.store(false, Relaxed);
        self.active_downloads.store(0, Relaxed);
    }

    /// Fetches domains for which the node currently has assigned cookies.
    ///
    /// # Errors
    ///
    /// Returns an error if auth metadata cannot be built or the RPC fails.
    pub async fn fetch_supported_domains(&self) -> Result<Vec<String>, NodeHandleError> {
        let mut client = NodeCapabilitiesClient::new(self.channel.clone());
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            client.get_supported_domains(authenticated_request(Empty {}, &self.token)?),
        )
        .await
        .map_err(|_| tonic::Status::unavailable("Node capabilities timed out"))??;
        Ok(response.into_inner().domains_with_cookies)
    }

    /// Fetches the node active-download and capacity counters.
    ///
    /// # Errors
    ///
    /// Returns an error if auth metadata cannot be built or the RPC fails.
    pub async fn fetch_status(&self) -> Result<(u32, u32), NodeHandleError> {
        let mut client = NodeCapabilitiesClient::new(self.channel.clone());
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            client.get_status(authenticated_request(Empty {}, &self.token)?),
        )
        .await
        .map_err(|_| tonic::Status::unavailable("Node status timed out"))??;
        let status = response.into_inner();
        Ok((status.active_downloads, status.max_concurrent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(capacity: u32) -> NodeHandle {
        let node = NodeHandle::new(
            "test".into(),
            "http://127.0.0.1:1".into(),
            "test".into(),
            tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy(),
        );
        node.update_remote_status(0, capacity);
        node
    }

    #[tokio::test]
    async fn remote_status_is_the_only_load_sample() {
        let node = node(1);
        assert!(node.is_available());
        assert_eq!(node.active_downloads(), 0);
        node.update_remote_status(1, 1);
        assert_eq!(node.active_downloads(), 1);
        assert_eq!(node.max_concurrent(), 1);
        node.mark_unavailable();
        assert!(!node.is_available());
    }
}
