use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
    time::Duration,
};

use futures_util::{stream, StreamExt as _};
use tokio::sync::watch;
use tracing::{info, warn};

use crate::{
    client::{DownloaderServiceTarget, DownloaderTlsConfig, NodeClient},
    handle::NodeHandle,
    outcome::record_execution_progress,
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
    capacity: watch::Sender<Capacity>,
}

/// Observational dispatch budget, never a reservation or admission decision.
#[derive(Clone, Copy, Debug, Default)]
pub struct Capacity {
    pub available: usize,
    pub total: usize,
}

impl Capacity {
    #[must_use]
    pub fn dispatch_budget(self, running: usize) -> usize {
        self.available.min(self.total.saturating_sub(running))
    }
}

impl NodeRouter {
    #[cfg(test)]
    pub(crate) fn with_test_nodes(nodes: Vec<Arc<NodeHandle>>) -> (Self, watch::Sender<Capacity>) {
        let (capacity, _) = watch::channel(Self::capacity_hint(&nodes));
        (
            Self {
                nodes: RwLock::new(nodes),
                domain_cookie_map: RwLock::new(HashMap::new()),
                topology_refresh: tokio::sync::Mutex::new(()),
                max_file_size: 1024,
                client: NodeClient::default(),
                service_target: Arc::new(DownloaderServiceTarget {
                    host: "localhost".into(),
                    port: 1,
                }),
                node_token: "test".into(),
                capacity: capacity.clone(),
            },
            capacity,
        )
    }

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
            capacity: watch::channel(Capacity::default()).0,
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
    pub fn subscribe_capacity(&self) -> watch::Receiver<Capacity> {
        self.capacity.subscribe()
    }

    /// Callers mark the current sample seen before attempting admission, so a
    /// refresh during that attempt remains observable here.
    pub(crate) async fn wait_for_status_change(capacity: &mut watch::Receiver<Capacity>) -> bool {
        record_execution_progress();
        loop {
            tokio::select! {
                changed = capacity.changed() => {
                    if changed.is_err() { return false; }
                    record_execution_progress();
                    return true;
                }
                () = tokio::time::sleep(Duration::from_secs(30)) => {
                    // Waiting for admission is scheduler activity, not downloader progress.
                    record_execution_progress();
                }
            }
        }
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
        self.capacity.send_replace(Self::capacity_hint(&self.nodes()));
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

    fn capacity_hint(nodes: &[Arc<NodeHandle>]) -> Capacity {
        nodes
            .iter()
            .filter(|node| node.is_available())
            .fold(Capacity::default(), |mut sum, node| {
                sum.total += node.max_concurrent() as usize;
                sum.available += node.max_concurrent().saturating_sub(node.active_downloads()) as usize;
                sum
            })
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

    fn node(active: u32, maximum: u32) -> Arc<NodeHandle> {
        let node = Arc::new(NodeHandle::new(
            "test".into(),
            "http://127.0.0.1:1".into(),
            "test".into(),
            tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy(),
        ));
        node.update_remote_status(active, maximum);
        node
    }

    #[tokio::test]
    async fn dispatch_hint_uses_all_healthy_node_capacity() {
        let unavailable = node(0, 100);
        unavailable.mark_unavailable();
        let hint = NodeRouter::capacity_hint(&[node(3, 5), node(1, 10), unavailable]);
        assert_eq!((hint.available, hint.total), (11, 15));
        assert_eq!(hint.dispatch_budget(0), 11);
        assert_eq!(hint.dispatch_budget(14), 1);
    }

    #[tokio::test]
    async fn large_topology_has_no_fixed_dispatch_ceiling() {
        let nodes = (0..50).map(|_| node(0, 5)).collect::<Vec<_>>();
        let hint = NodeRouter::capacity_hint(&nodes);
        assert_eq!(hint.dispatch_budget(0), 250);
        assert_eq!(hint.dispatch_budget(250), 0);
    }

    #[tokio::test]
    async fn full_or_overreported_nodes_provide_no_free_dispatch_budget() {
        let hint = NodeRouter::capacity_hint(&[node(5, 5), node(12, 10)]);
        assert_eq!(hint.available, 0);
        assert_eq!(hint.dispatch_budget(0), 0);
    }

    #[tokio::test]
    async fn admission_wait_observes_status_that_arrived_after_attempt_started() {
        let (status, mut capacity) = watch::channel(Capacity::default());
        capacity.borrow_and_update();
        status.send_replace(Capacity { available: 1, total: 1 });

        assert!(
            tokio::time::timeout(Duration::from_millis(100), NodeRouter::wait_for_status_change(&mut capacity))
                .await
                .unwrap()
        );
    }

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
