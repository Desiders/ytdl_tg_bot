use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU32, Ordering::Relaxed},
        Arc, RwLock,
    },
};

use anyhow::{Context, Result};
use tonic::{
    metadata::{Ascii, MetadataValue},
    transport::{Channel, Endpoint},
    Request,
};
use tracing::{error, warn};
use ytdl_tg_bot_proto::downloader::{node_capabilities_client::NodeCapabilitiesClient, Empty};

use crate::config::DownloadNodeConfig;

pub struct NodeHandle {
    pub address: Box<str>,
    pub token: Box<str>,
    pub max_concurrent: u32,
    local_active_downloads: AtomicU32,
    remote_active_downloads: AtomicU32,
    pub channel: Channel,
}

pub struct NodeRouter {
    nodes: Vec<Arc<NodeHandle>>,
    domain_cookie_map: RwLock<HashMap<String, Vec<usize>>>,
    max_file_size: u64,
}

impl NodeRouter {
    pub async fn new(configs: &[DownloadNodeConfig], max_file_size: u64) -> Result<Self> {
        let mut nodes = Vec::with_capacity(configs.len());
        let mut domain_cookie_map = HashMap::new();

        for (index, config) in configs.iter().enumerate() {
            let channel = Endpoint::from_shared(config.address.to_string())
                .with_context(|| format!("Invalid node address {}", config.address))?
                .connect_lazy();
            let node = Arc::new(NodeHandle {
                address: config.address.clone(),
                token: config.token.clone(),
                max_concurrent: config.max_concurrent,
                local_active_downloads: AtomicU32::new(0),
                remote_active_downloads: AtomicU32::new(0),
                channel,
            });

            match fetch_supported_domains(&node).await {
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

    #[must_use]
    pub fn max_file_size(&self) -> u64 {
        self.max_file_size
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn pick_node(&self, domain: Option<&str>) -> Option<Arc<NodeHandle>> {
        self.pick_node_excluding(domain, &HashSet::new())
    }

    #[must_use]
    pub fn pick_node_excluding(&self, domain: Option<&str>, excluded: &HashSet<String>) -> Option<Arc<NodeHandle>> {
        let normalized_domain = domain.map(|value| value.trim_start_matches("www."));
        let domain_candidates = normalized_domain
            .and_then(|domain| self.domain_cookie_map.read().ok().and_then(|map| map.get(domain).cloned()))
            .unwrap_or_default();

        if let Some(index) = self.select_best_index(domain_candidates, excluded) {
            return self.nodes.get(index).cloned();
        }

        self.select_best_index((0..self.nodes.len()).collect(), excluded)
            .and_then(|index| self.nodes.get(index).cloned())
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
            match fetch_supported_domains(node).await {
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

pub fn authenticated_request<T>(message: T, token: &str) -> std::result::Result<Request<T>, tonic::metadata::errors::InvalidMetadataValue> {
    let mut request = Request::new(message);
    let value: MetadataValue<Ascii> = format!("Bearer {token}").parse()?;
    request.metadata_mut().insert("authorization", value);
    Ok(request)
}

fn select_best_index(candidates: Vec<NodeSnapshot<'_>>) -> Option<usize> {
    candidates.into_iter().min_by(compare_nodes).map(|node| node.index)
}

fn compare_nodes(left: &NodeSnapshot<'_>, right: &NodeSnapshot<'_>) -> Ordering {
    let left_active = left.estimated_active_downloads;
    let right_active = right.estimated_active_downloads;

    // Prefer the node that will have the lower utilization after taking this request.
    let left_projected = (left_active + 1) * right.max_concurrent;
    let right_projected = (right_active + 1) * left.max_concurrent;

    left_projected
        .cmp(&right_projected)
        .then_with(|| left_active.cmp(&right_active))
        .then_with(|| right.max_concurrent.cmp(&left.max_concurrent))
        .then_with(|| left.address.cmp(right.address))
}

#[derive(Clone, Copy)]
struct NodeSnapshot<'a> {
    index: usize,
    address: &'a str,
    max_concurrent: u32,
    estimated_active_downloads: u32,
}

async fn fetch_supported_domains(node: &NodeHandle) -> Result<Vec<String>> {
    let mut client = NodeCapabilitiesClient::new(node.channel.clone());
    let response = client.get_supported_domains(authenticated_request(Empty {}, &node.token)?).await?;
    Ok(response.into_inner().domains_with_cookies)
}

impl NodeRouter {
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

impl NodeHandle {
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
