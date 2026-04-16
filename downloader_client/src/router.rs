use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

use proto::downloader::{node_capabilities_client::NodeCapabilitiesClient, Empty};
use tracing::{error, info, warn};

use crate::{
    authenticated_request,
    client::{DownloaderServiceTarget, DownloaderTlsConfig, NodeClient},
    handle::NodeHandle,
    selection::{select_best_index, NodeSnapshot},
};

#[derive(Clone, Debug)]
pub struct DownloaderClusterConfig {
    pub node_token: Box<str>,
    pub tls: DownloaderTlsConfig,
}

pub struct NodeRouter {
    nodes: RwLock<Vec<Arc<NodeHandle>>>,
    domain_cookie_map: RwLock<HashMap<String, Vec<usize>>>,
    max_file_size: u64,
    client: NodeClient,
    service_target: Arc<DownloaderServiceTarget>,
    node_token: Box<str>,
}

impl NodeRouter {
    pub fn new(config: &DownloaderClusterConfig, max_file_size: u64, service_target: Arc<DownloaderServiceTarget>) -> Self {
        let client = NodeClient::load(&config.tls, service_target.host.as_ref());

        Self {
            nodes: RwLock::new(Vec::new()),
            domain_cookie_map: RwLock::new(HashMap::new()),
            max_file_size,
            client,
            service_target,
            node_token: config.node_token.clone(),
        }
    }

    #[must_use]
    pub const fn max_file_size(&self) -> u64 {
        self.max_file_size
    }

    #[must_use]
    pub fn nodes(&self) -> Vec<Arc<NodeHandle>> {
        self.nodes.read().map(|nodes| nodes.clone()).unwrap_or_default()
    }

    #[must_use]
    pub fn pick_node(&self, domain: Option<&str>, excluded: &HashSet<String>) -> Option<Arc<NodeHandle>> {
        let nodes = self.nodes();
        let normalized_domain = domain.map(|value| value.trim_start_matches("www."));
        let domain_candidates = normalized_domain
            .and_then(|value| self.domain_cookie_map.read().ok().and_then(|map| map.get(value).cloned()))
            .unwrap_or_default();

        if let Some(index) = self.select_best_index(&nodes, domain_candidates, excluded) {
            return nodes.get(index).cloned();
        }

        self.select_best_index(&nodes, (0..nodes.len()).collect(), excluded)
            .and_then(|index| nodes.get(index).cloned())
    }

    pub async fn refresh_status(&self) {
        self.refresh_nodes().await;

        for node in self.nodes() {
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
                    let status = response.into_inner();
                    node.update_remote_status(status.active_downloads, status.max_concurrent);
                }
                Err(err) => {
                    node.mark_unavailable();
                    warn!(node = %node.address, error = %err, "Failed to refresh node status");
                }
            }
        }
    }

    pub async fn refresh_capabilities(&self) {
        self.refresh_nodes().await;

        let mut domain_cookie_map = HashMap::new();
        for (index, node) in self.nodes().into_iter().enumerate() {
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

    async fn refresh_nodes(&self) {
        let node_addresses = match self.service_target.resolve_nodes().await {
            Ok(nodes) => nodes,
            Err(err) => {
                warn!(dns = %self.service_target.authority(), error = %err, "Failed to resolve downloader service DNS");
                return;
            }
        };

        if node_addresses.is_empty() {
            warn!(dns = %self.service_target.authority(), "DNS lookup returned no downloader endpoints");
            return;
        }

        let existing = self
            .nodes()
            .into_iter()
            .map(|node| node.address.to_string())
            .collect::<HashSet<_>>();
        let next = node_addresses.iter().map(|addr| format!("https://{addr}")).collect::<HashSet<_>>();
        if existing == next {
            return;
        }

        let mut nodes = Vec::with_capacity(node_addresses.len());
        let mut domain_cookie_map = HashMap::new();

        for (index, address) in node_addresses.into_iter().enumerate() {
            let address = format!("https://{address}");
            let channel = match self.client.build_channel(&address) {
                Ok(channel) => channel,
                Err(err) => {
                    warn!(node = %address, error = %err, "Failed to initialize node channel");
                    continue;
                }
            };

            let node = Arc::new(NodeHandle::new(
                format!("downloader-{}", index + 1).into_boxed_str(),
                address.into_boxed_str(),
                self.node_token.clone(),
                channel,
            ));

            if let Ok((active_downloads, max_concurrent)) = node.fetch_status().await {
                node.update_remote_status(active_downloads, max_concurrent);
            } else {
                node.mark_unavailable();
                warn!(node = %node.address, "Node marked unavailable during refresh_nodes bootstrap");
            }

            match node.fetch_supported_domains().await {
                Ok(domains) => {
                    for domain in domains {
                        domain_cookie_map.entry(domain).or_insert_with(Vec::new).push(nodes.len());
                    }
                }
                Err(err) => {
                    warn!(node = %node.address, error = %err, "Failed to fetch node capabilities");
                }
            }

            nodes.push(node);
        }

        if nodes.is_empty() {
            warn!(dns = %self.service_target.authority(), "No downloader nodes passed initialization during refresh");
            return;
        }

        info!(dns = %self.service_target.authority(), node_count = nodes.len(), "Refreshed downloader nodes from DNS");
        self.replace_nodes(nodes, domain_cookie_map);
    }

    fn replace_nodes(&self, nodes: Vec<Arc<NodeHandle>>, domain_cookie_map: HashMap<String, Vec<usize>>) {
        if let Ok(mut lock) = self.nodes.write() {
            *lock = nodes;
        }
        if let Ok(mut map) = self.domain_cookie_map.write() {
            *map = domain_cookie_map;
        }
    }

    fn select_best_index(&self, nodes: &[Arc<NodeHandle>], indices: Vec<usize>, excluded: &HashSet<String>) -> Option<usize> {
        select_best_index(self.select_candidates(nodes, indices, excluded))
    }

    fn select_candidates<'a>(
        &self,
        nodes: &'a [Arc<NodeHandle>],
        indices: Vec<usize>,
        excluded: &HashSet<String>,
    ) -> Vec<NodeSnapshot<'a>> {
        indices
            .into_iter()
            .filter_map(|index| nodes.get(index).map(|node| (index, node)))
            .filter(|(_, node)| !excluded.contains(node.address.as_ref()))
            .filter(|(_, node)| node.has_capacity())
            .map(|(index, node)| NodeSnapshot {
                index,
                address: node.address.as_ref(),
                max_concurrent: node.max_concurrent(),
                estimated_active_downloads: node.estimated_active_downloads(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::selection::NodeSnapshot;

    use super::*;

    fn make_snapshot(index: usize, address: &'static str, max_concurrent: u32, estimated_active_downloads: u32) -> NodeSnapshot<'static> {
        NodeSnapshot {
            index,
            address,
            max_concurrent,
            estimated_active_downloads,
        }
    }

    #[test]
    fn prefers_lower_projected_utilization() {
        let selected = select_best_index(vec![make_snapshot(0, "a", 1, 0), make_snapshot(1, "b", 4, 2)]);
        assert_eq!(selected, Some(1));
    }

    #[test]
    fn tie_breaks_by_lower_active_downloads_then_capacity_then_address() {
        let selected = select_best_index(vec![
            make_snapshot(0, "b", 2, 1),
            make_snapshot(1, "a", 2, 1),
            make_snapshot(2, "c", 1, 0),
        ]);
        assert_eq!(selected, Some(2));
    }
}
