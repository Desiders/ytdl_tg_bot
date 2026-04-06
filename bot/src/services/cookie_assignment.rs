use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::{Arc, Mutex},
};
use tracing::{info, warn};

use super::{
    fs::cookies::load_cookies_from_directory,
    node_router::{NodeHandle, NodeRouter},
};

const COOKIE_DIR: &str = "/app/cookies";

pub struct CookieAssignmentService {
    pub node_router: Arc<NodeRouter>,
    assignments: Mutex<HashMap<String, String>>, // cookie_id -> worker_id
}

impl CookieAssignmentService {
    pub fn new(node_router: Arc<NodeRouter>) -> Self {
        Self {
            node_router,
            assignments: Mutex::new(HashMap::new()),
        }
    }

    pub async fn sync_cycle(&self) {
        self.node_router.refresh_status().await;

        let nodes = self.node_router.nodes();
        let mut available_workers = HashMap::new();
        let mut worker_cookie_ids = HashMap::new();

        for node in &nodes {
            if !node.is_available() {
                continue;
            }

            let worker_id = node.address.as_ref();
            match node.list_node_cookies().await {
                Ok(cookie_ids) => {
                    worker_cookie_ids.insert(worker_id, cookie_ids.into_iter().collect::<HashSet<_>>());
                    available_workers.insert(worker_id, node.clone());
                }
                Err(err) => {
                    node.mark_unavailable();
                    warn!(node = %node.address, error = %err, "Failed to list worker cookies");
                }
            }
        }

        self.reconcile_assignments(&available_workers, &worker_cookie_ids).await;
        self.assign_free_cookies(&available_workers).await;

        self.node_router.refresh_capabilities().await;
    }

    async fn reconcile_assignments(
        &self,
        available_workers: &HashMap<&str, Arc<NodeHandle>>,
        worker_cookie_ids: &HashMap<&str, HashSet<String>>,
    ) {
        let mut stale_cookie_count = 0usize;
        let mut stale_cookie_remove_failed = 0usize;
        let assigned_cookie_ids = {
            let Some(mut assignments) = self.assignments.lock().ok() else {
                return;
            };

            let available_worker_ids = available_workers.keys().map(|id| &**id).collect::<HashSet<_>>();
            assignments.retain(|_, worker_id| available_worker_ids.contains(worker_id.as_str()));

            // Drop assignments not present on worker anymore.
            assignments.retain(|cookie_id, worker_id| {
                worker_cookie_ids
                    .get(worker_id.as_str())
                    .is_some_and(|cookie_ids| cookie_ids.contains(cookie_id))
            });

            assignments.keys().cloned().collect::<HashSet<_>>()
        };

        for (worker_id, cookie_ids) in worker_cookie_ids {
            let Some(worker) = available_workers.get(worker_id) else {
                continue;
            };
            for cookie_id in cookie_ids {
                if assigned_cookie_ids.contains(cookie_id) {
                    continue;
                }
                stale_cookie_count += 1;
                if let Err(err) = worker.remove_cookie(cookie_id).await {
                    stale_cookie_remove_failed += 1;
                    warn!(node = %worker.address, cookie_id = %cookie_id, error = %err, "Failed to remove stale cookie from worker");
                }
            }
        }

        if stale_cookie_count > 0 || stale_cookie_remove_failed > 0 {
            info!(
                stale_cookie_count,
                stale_cookie_remove_failed,
                "Cookie assignment reconcile completed"
            );
        }
    }

    async fn assign_free_cookies(&self, available_workers: &HashMap<&str, Arc<NodeHandle>>) {
        let cookies = match load_cookies_from_directory(Path::new(COOKIE_DIR)) {
            Ok(cookies) => cookies,
            Err(err) => {
                warn!(cookie_dir = COOKIE_DIR, error = %err, "Failed to load cookie files");
                return;
            }
        };

        let (assigned_cookies, mut worker_domains) = {
            let Some(assignments) = self.assignments.lock().ok() else {
                return;
            };

            let cookie_domains = cookies
                .iter()
                .map(|cookie| (cookie.cookie_id.as_str(), cookie.domain.as_str()))
                .collect::<HashMap<_, _>>();

            let mut worker_domains: HashMap<_, HashSet<_>> = HashMap::new();
            for (cookie_id, worker_id) in assignments.iter() {
                if let Some(&domain) = cookie_domains.get(cookie_id.as_str()) {
                    worker_domains.entry(worker_id.clone()).or_default().insert(domain);
                }
            }

            (assignments.keys().cloned().collect::<HashSet<_>>(), worker_domains)
        };

        let mut free_cookies = cookies
            .iter()
            .filter(|cookie| !assigned_cookies.contains(cookie.cookie_id.as_str()))
            .collect::<Vec<_>>();
        free_cookies.sort_by(|a, b| a.cookie_id.cmp(&b.cookie_id));

        let mut workers = available_workers.values().collect::<Vec<_>>();
        workers.sort_by(|a, b| a.address.cmp(&b.address));
        let mut assigned_count = 0usize;
        let mut read_failed_count = 0usize;
        let mut push_failed_count = 0usize;
        let mut skipped_by_domain_constraint = 0usize;

        for cookie in free_cookies {
            let Some(worker) = workers.iter().find(|worker| {
                !worker_domains
                    .get(worker.address.as_ref())
                    .is_some_and(|domains| domains.contains(cookie.domain.as_str()))
            }) else {
                skipped_by_domain_constraint += 1;
                continue;
            };

            let data = match fs::read_to_string(&cookie.path) {
                Ok(data) => data,
                Err(err) => {
                    read_failed_count += 1;
                    warn!(cookie_id = %cookie.cookie_id, path = %cookie.path.display(), error = %err, "Failed to read cookie file");
                    continue;
                }
            };

            if let Err(err) = worker.push_cookie(&cookie.cookie_id, &cookie.domain, &data).await {
                push_failed_count += 1;
                warn!(
                    node = %worker.address,
                    cookie_id = %cookie.cookie_id,
                    domain = %cookie.domain,
                    error = %err,
                    "Failed to push cookie to worker"
                );
                continue;
            }

            let worker_id = worker.address.to_string();
            if let Ok(mut assignments) = self.assignments.lock() {
                assignments.insert(cookie.cookie_id.clone(), worker_id.clone());
            }
            worker_domains.entry(worker_id).or_default().insert(cookie.domain.as_str());
            assigned_count += 1;
            info!(node = %worker.address, cookie_id = %cookie.cookie_id, domain = %cookie.domain, "Cookie assigned and pushed");
        }

        if assigned_count > 0
            || read_failed_count > 0
            || push_failed_count > 0
            || skipped_by_domain_constraint > 0
        {
            info!(
                assigned_count,
                read_failed_count,
                push_failed_count,
                skipped_by_domain_constraint,
                "Cookie assignment free cookies processed"
            );
        }
    }
}
