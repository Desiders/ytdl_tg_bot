use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

use futures_util::{stream, StreamExt as _};
use tracing::{info, warn};

use crate::{
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
    domain_cookie_map: RwLock<HashMap<String, Vec<String>>>,
    topology_refresh: tokio::sync::Mutex<()>,
    max_file_size: u64,
    client: NodeClient,
    service_target: Arc<DownloaderServiceTarget>,
    node_token: Box<str>,
}

impl NodeRouter {
    #[must_use]
    pub fn new(config: &DownloaderClusterConfig, max_file_size: u64, service_target: Arc<DownloaderServiceTarget>) -> Self {
        let client = NodeClient::load(&config.tls, service_target.host.as_ref());

        Self {
            nodes: RwLock::new(Vec::new()),
            domain_cookie_map: RwLock::new(HashMap::new()),
            topology_refresh: tokio::sync::Mutex::new(()),
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

        let indices = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| domain_candidates.iter().any(|address| address == node.address.as_ref()))
            .map(|(index, _)| index)
            .collect();
        if let Some(index) = Self::select_best_index(&nodes, indices, excluded) {
            return nodes.get(index).cloned();
        }

        Self::select_best_index(&nodes, (0..nodes.len()).collect(), excluded).and_then(|index| nodes.get(index).cloned())
    }

    pub async fn refresh_status(&self) {
        self.refresh_nodes().await;
        stream::iter(self.nodes())
            .for_each_concurrent(Some(16), |node| async move {
                match node.fetch_status().await {
                    Ok((active, maximum)) => {
                        node.update_remote_status(active, maximum);
                    }
                    Err(err) => {
                        node.mark_unavailable();
                        warn!(node = %node.address, error = %err, "Failed to refresh node status");
                    }
                }
            })
            .await;
    }

    pub async fn refresh_capabilities(&self) {
        self.refresh_nodes().await;
        let domains = stream::iter(self.nodes())
            .map(|node| async move {
                match node.fetch_supported_domains().await {
                    Ok(domains) => Some((node.address.to_string(), domains)),
                    Err(err) => {
                        warn!(node = %node.address, error = %err, "Failed to refresh node capabilities");
                        None
                    }
                }
            })
            .buffer_unordered(16)
            .collect::<Vec<_>>()
            .await;
        let mut domain_cookie_map = HashMap::new();
        for (address, domains) in domains.into_iter().flatten() {
            for domain in domains {
                domain_cookie_map.entry(domain).or_insert_with(Vec::new).push(address.clone());
            }
        }

        if let Ok(mut map) = self.domain_cookie_map.write() {
            *map = domain_cookie_map;
        }
    }

    async fn refresh_nodes(&self) {
        let _refresh = self.topology_refresh.lock().await;
        let node_addresses = match self.service_target.resolve_nodes().await {
            Ok(nodes) => nodes,
            Err(err) => {
                warn!(dns = %self.service_target.authority(), error = %err, "Failed to resolve downloader service DNS");
                return;
            }
        };

        if node_addresses.is_empty() {
            warn!(dns = %self.service_target.authority(), "DNS lookup returned no downloader endpoints");
            self.replace_nodes(Vec::new(), HashMap::new());
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
        let previous: HashMap<_, _> = self.nodes().into_iter().map(|node| (node.address.to_string(), node)).collect();

        for (index, address) in node_addresses.into_iter().enumerate() {
            let address = format!("https://{address}");
            let channel = match self.client.build_channel(&address) {
                Ok(channel) => channel,
                Err(err) => {
                    warn!(node = %address, error = %err, "Failed to initialize node channel");
                    continue;
                }
            };

            let node = previous.get(&address).cloned().unwrap_or_else(|| {
                Arc::new(NodeHandle::new(
                    format!("downloader-{}", index + 1).into_boxed_str(),
                    address.into_boxed_str(),
                    self.node_token.clone(),
                    channel,
                ))
            });

            nodes.push(node);
        }

        if nodes.is_empty() {
            warn!(dns = %self.service_target.authority(), "No downloader nodes passed initialization during refresh");
            return;
        }

        info!(dns = %self.service_target.authority(), node_count = nodes.len(), "Refreshed downloader nodes from DNS");
        self.replace_nodes(nodes, HashMap::new());
    }

    fn replace_nodes(&self, nodes: Vec<Arc<NodeHandle>>, domain_cookie_map: HashMap<String, Vec<String>>) {
        if let Ok(mut lock) = self.nodes.write() {
            *lock = nodes;
        }
        if let Ok(mut map) = self.domain_cookie_map.write() {
            *map = domain_cookie_map;
        }
    }

    fn select_best_index(nodes: &[Arc<NodeHandle>], indices: Vec<usize>, excluded: &HashSet<String>) -> Option<usize> {
        select_best_index(Self::select_candidates(nodes, indices, excluded))
    }

    fn select_candidates<'a>(nodes: &'a [Arc<NodeHandle>], indices: Vec<usize>, excluded: &HashSet<String>) -> Vec<NodeSnapshot<'a>> {
        indices
            .into_iter()
            .filter_map(|index| nodes.get(index).map(|node| (index, node)))
            .filter(|(_, node)| !excluded.contains(node.address.as_ref()))
            .filter(|(_, node)| node.is_available())
            .map(|(index, node)| NodeSnapshot {
                index,
                address: node.address.as_ref(),
                max_concurrent: node.max_concurrent(),
                estimated_active_downloads: node.active_downloads(),
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

    #[tokio::test]
    async fn cached_full_load_is_ranked_but_not_excluded_from_admission() {
        let node = Arc::new(NodeHandle::new(
            "test".into(),
            "http://127.0.0.1:1".into(),
            "test".into(),
            tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy(),
        ));
        node.update_remote_status(1, 1);
        let nodes = [node];
        let candidates = NodeRouter::select_candidates(&nodes, vec![0], &HashSet::new());
        assert_eq!(candidates.len(), 1);
    }
}
