mod auth;
mod handle;
mod selection;
mod tls;

pub use auth::authenticated_request;
pub use handle::NodeHandle;

use anyhow::Result;
use selection::{select_best_index, NodeSnapshot};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};
use tls::{build_channel, DownloadClientTls};
use tracing::{error, warn};
use ytdl_tg_bot_proto::downloader::{node_capabilities_client::NodeCapabilitiesClient, Empty};

use crate::config::{DownloadNodeConfig, DownloadTlsConfig};

pub struct NodeRouter {
    nodes: Vec<Arc<NodeHandle>>,
    domain_cookie_map: RwLock<HashMap<String, Vec<usize>>>,
    max_file_size: u64,
}

impl NodeRouter {
    pub async fn new(configs: &[DownloadNodeConfig], tls_config: Option<&DownloadTlsConfig>, max_file_size: u64) -> Result<Self> {
        let mut nodes = Vec::with_capacity(configs.len());
        let mut domain_cookie_map = HashMap::new();
        let tls_material = DownloadClientTls::load(tls_config)?;

        for (index, config) in configs.iter().enumerate() {
            let channel = build_channel(config, tls_material.as_ref())?;
            let node = Arc::new(NodeHandle::new(
                config.address.clone(),
                config.token.clone(),
                config.max_concurrent,
                channel,
            ));

            match node.fetch_supported_domains().await {
                Ok(domains) => {
                    for domain in domains {
                        domain_cookie_map.entry(domain).or_insert_with(Vec::new).push(index);
                    }
                }
                Err(err) => {
                    warn!(node = %node.address, error = %err, "Failed to fetch node capabilities");
                }
            }

            nodes.push(node);
        }

        Ok(Self {
            nodes,
            domain_cookie_map: RwLock::new(domain_cookie_map),
            max_file_size,
        })
    }

    #[inline]
    #[must_use]
    pub const fn max_file_size(&self) -> u64 {
        self.max_file_size
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn pick_node(&self, domain: Option<&str>) -> Option<&NodeHandle> {
        self.pick_node_excluding(domain, &HashSet::new())
    }

    #[must_use]
    pub fn pick_node_excluding(&self, domain: Option<&str>, excluded: &HashSet<String>) -> Option<&NodeHandle> {
        let normalized_domain = domain.map(|value| value.trim_start_matches("www."));
        let domain_candidates = normalized_domain
            .and_then(|domain| self.domain_cookie_map.read().ok().and_then(|map| map.get(domain).cloned()))
            .unwrap_or_default();

        if let Some(index) = self.select_best_index(domain_candidates, excluded) {
            return self.nodes.get(index).map(AsRef::as_ref);
        }

        self.select_best_index((0..self.nodes.len()).collect(), excluded)
            .and_then(|index| self.nodes.get(index).map(AsRef::as_ref))
    }

    pub async fn refresh_status(&self) {
        for node in &self.nodes {
            let mut client = NodeCapabilitiesClient::new(node.channel.clone());
            let request = match authenticated_request(Empty {}, &node.token) {
                Ok(request) => request,
                Err(err) => {
                    error!(node = %node.address, error = %err, "Failed to build node status request");
                    continue;
                }
            };

            match client.get_status(request).await {
                Ok(response) => {
                    node.update_remote_active_downloads(response.into_inner().active_downloads);
                }
                Err(err) => {
                    warn!(node = %node.address, error = %err, "Failed to refresh node status");
                }
            }
        }
    }

    pub async fn refresh_capabilities(&self) {
        let mut domain_cookie_map = HashMap::new();

        for (index, node) in self.nodes.iter().enumerate() {
            match node.fetch_supported_domains().await {
                Ok(domains) => {
                    for domain in domains {
                        domain_cookie_map.entry(domain).or_insert_with(Vec::new).push(index);
                    }
                }
                Err(err) => {
                    warn!(node = %node.address, error = %err, "Failed to refresh node capabilities");
                }
            }
        }

        if let Ok(mut map) = self.domain_cookie_map.write() {
            *map = domain_cookie_map;
        }
    }
}

impl NodeRouter {
    #[inline]
    fn select_best_index(&self, indices: Vec<usize>, excluded: &HashSet<String>) -> Option<usize> {
        select_best_index(self.select_candidates(indices, excluded))
    }

    fn select_candidates(&self, indices: Vec<usize>, excluded: &HashSet<String>) -> Vec<NodeSnapshot<'_>> {
        indices
            .into_iter()
            .filter_map(|index| self.nodes.get(index).map(|node| (index, node)))
            .filter(|(_, node)| !excluded.contains(node.address.as_ref()))
            .filter(|(_, node)| node.has_capacity())
            .map(|(index, node)| NodeSnapshot {
                index,
                address: node.address.as_ref(),
                max_concurrent: node.max_concurrent,
                estimated_active_downloads: node.estimated_active_downloads(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot(index: usize, address: &'static str, max_concurrent: u32, estimated_active_downloads: u32) -> NodeSnapshot<'static> {
        NodeSnapshot {
            index,
            address,
            max_concurrent,
            estimated_active_downloads,
        }
    }

    #[tokio::test]
    async fn select_best_node_prefers_lower_projected_utilization() {
        let smaller = make_snapshot(0, "node-a", 2, 1);
        let larger = make_snapshot(1, "node-b", 10, 4);

        let selected = select_best_index(vec![smaller, larger]).expect("Missing node");

        assert_eq!(selected, 1);
    }

    #[tokio::test]
    async fn select_best_node_prefers_lower_absolute_load_when_utilization_matches() {
        let lighter = make_snapshot(0, "node-a", 2, 0);
        let heavier = make_snapshot(1, "node-b", 4, 1);

        let selected = select_best_index(vec![heavier, lighter]).expect("Missing node");

        assert_eq!(selected, 0);
    }

    #[tokio::test]
    async fn select_best_node_prefers_larger_capacity_when_loads_match() {
        let smaller = make_snapshot(0, "node-a", 2, 0);
        let larger = make_snapshot(1, "node-b", 5, 0);

        let selected = select_best_index(vec![smaller, larger]).expect("Missing node");

        assert_eq!(selected, 1);
    }
}
