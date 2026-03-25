use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU32, Ordering},
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

        let candidates = self.select_candidates(domain_candidates, excluded);
        if let Some(node) = select_least_loaded(candidates) {
            return Some(node);
        }

        select_least_loaded(self.select_candidates((0..self.nodes.len()).collect(), excluded))
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

fn select_least_loaded(candidates: Vec<Arc<NodeHandle>>) -> Option<Arc<NodeHandle>> {
    candidates.into_iter().min_by_key(|node| node.estimated_active_downloads())
}

async fn fetch_supported_domains(node: &NodeHandle) -> Result<Vec<String>> {
    let mut client = NodeCapabilitiesClient::new(node.channel.clone());
    let response = client.get_supported_domains(authenticated_request(Empty {}, &node.token)?).await?;
    Ok(response.into_inner().domains_with_cookies)
}

impl NodeRouter {
    fn select_candidates(&self, indices: Vec<usize>, excluded: &HashSet<String>) -> Vec<Arc<NodeHandle>> {
        indices
            .into_iter()
            .filter_map(|index| self.nodes.get(index).cloned())
            .filter(|node| !excluded.contains(node.address.as_ref()))
            .filter(|node| node.has_capacity())
            .collect()
    }
}

impl NodeHandle {
    pub fn reserve_download_slot(&self) {
        self.local_active_downloads.fetch_add(1, Ordering::Relaxed);
    }

    pub fn release_download_slot(&self) {
        let mut current = self.local_active_downloads.load(Ordering::Relaxed);
        loop {
            let next = current.saturating_sub(1);
            match self
                .local_active_downloads
                .compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(actual) => current = actual,
            }
        }
    }

    pub fn update_remote_active_downloads(&self, active_downloads: u32) {
        self.remote_active_downloads.store(active_downloads, Ordering::Relaxed);
    }

    #[must_use]
    pub fn estimated_active_downloads(&self) -> u32 {
        self.local_active_downloads
            .load(Ordering::Relaxed)
            .max(self.remote_active_downloads.load(Ordering::Relaxed))
    }

    #[must_use]
    pub fn has_capacity(&self) -> bool {
        self.estimated_active_downloads() < self.max_concurrent
    }
}
