use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

use downloader_client::{AssignmentNodeClient, AssignmentNodeHandle, DownloaderServiceTarget};
use tracing::{info, warn};

use crate::cookies::{load_cookies_from_directory, CookieRecord};

const COOKIE_DIR: &str = "/app/cookies";

#[derive(Default)]
struct WorkerAssignments<'a> {
    domains: HashMap<String, HashSet<&'a str>>,
    cookie_counts: HashMap<String, usize>,
}

pub struct CookieAssignmentService {
    client: AssignmentNodeClient,
    service_target: DownloaderServiceTarget,
    assignments: HashMap<String, String>,
}

impl CookieAssignmentService {
    pub fn new(client: AssignmentNodeClient, service_target: DownloaderServiceTarget) -> Self {
        Self {
            client,
            service_target,
            assignments: HashMap::new(),
        }
    }

    pub async fn sync_cycle(&mut self) {
        let Some(cookies) = self.load_source_cookies() else {
            return;
        };
        let workers = self.load_workers().await;
        if workers.is_empty() {
            warn!("Skipping cookie assignment cycle because no downloader workers were resolved");
            return;
        }

        let listed_worker_cookie_ids = self.load_worker_cookie_ids(&workers).await;
        if listed_worker_cookie_ids.is_empty() {
            warn!("Skipping cookie assignment cycle because no worker cookie state could be loaded");
            return;
        }

        self.revoke_removed_source_cookies(&cookies, &workers, &listed_worker_cookie_ids)
            .await;
        self.reconcile_assignments(&workers, &listed_worker_cookie_ids).await;
        self.assign_free_cookies(&cookies, &workers, &listed_worker_cookie_ids).await;
    }

    async fn load_workers(&self) -> HashMap<String, AssignmentNodeHandle> {
        let node_addresses = match self.service_target.resolve_nodes().await {
            Ok(nodes) => nodes,
            Err(err) => {
                warn!(dns = %self.service_target.authority(), error = %err, "Failed to resolve downloader service DNS");
                return HashMap::new();
            }
        };

        if node_addresses.is_empty() {
            warn!(
                dns = %self.service_target.authority(),
                "DNS lookup returned no downloader endpoints"
            );
            return HashMap::new();
        }

        let mut workers = HashMap::new();
        for address in node_addresses {
            let worker = match self.client.build_handle(address) {
                Ok(worker) => worker,
                Err(err) => {
                    warn!(node = %address, error = %err, "Failed to initialize node channel");
                    continue;
                }
            };

            workers.insert(worker.address.to_string(), worker);
        }

        workers
    }

    async fn load_worker_cookie_ids(&self, available_workers: &HashMap<String, AssignmentNodeHandle>) -> HashMap<String, HashSet<String>> {
        let mut worker_cookie_ids = HashMap::new();

        for worker in available_workers.values() {
            if let Err(err) = worker.fetch_status().await {
                warn!(node = %worker.address, error = %err, "Failed to refresh node status");
                continue;
            }

            match worker.list_node_cookies().await {
                Ok(cookie_ids) => {
                    worker_cookie_ids.insert(worker.address.to_string(), cookie_ids.into_iter().collect::<HashSet<_>>());
                }
                Err(err) => {
                    warn!(node = %worker.address, error = %err, "Failed to list worker cookies");
                }
            }
        }

        worker_cookie_ids
    }

    async fn reconcile_assignments(
        &mut self,
        available_workers: &HashMap<String, AssignmentNodeHandle>,
        worker_cookie_ids: &HashMap<String, HashSet<String>>,
    ) {
        let available_worker_ids = available_workers.keys().map(String::as_str).collect::<HashSet<_>>();
        self.assignments
            .retain(|_, worker_id| available_worker_ids.contains(worker_id.as_str()));
        self.assignments.retain(|cookie_id, worker_id| {
            worker_cookie_ids
                .get(worker_id)
                .map_or(true, |cookie_ids| cookie_ids.contains(cookie_id))
        });

        let assigned_cookie_ids = self.assignments.keys().cloned().collect::<HashSet<_>>();
        let mut stale_cookie_count = 0usize;
        let mut stale_cookie_remove_failed = 0usize;

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
                stale_cookie_remove_failed, "Cookie assignment reconcile completed"
            );
        }
    }

    async fn assign_free_cookies(
        &mut self,
        cookies: &[CookieRecord],
        available_workers: &HashMap<String, AssignmentNodeHandle>,
        worker_cookie_ids: &HashMap<String, HashSet<String>>,
    ) {
        let cookie_domains = self.cookie_domains(cookies);
        let assigned_cookies = self.assignments.keys().cloned().collect::<HashSet<_>>();
        let mut worker_assignments = self.build_worker_assignments(&cookie_domains);

        let mut free_cookies = cookies
            .iter()
            .filter(|cookie| !assigned_cookies.contains(cookie.cookie_id.as_str()))
            .collect::<Vec<_>>();
        free_cookies.sort_by(|a, b| a.cookie_id.cmp(&b.cookie_id));

        let mut workers = available_workers
            .iter()
            .filter_map(|(worker_id, worker)| worker_cookie_ids.contains_key(worker_id).then_some(worker))
            .collect::<Vec<_>>();
        workers.sort_by(|a, b| a.address.cmp(&b.address));

        let mut assigned_count = 0usize;
        let mut read_failed_count = 0usize;
        let mut push_failed_count = 0usize;
        let mut skipped_by_domain_constraint = 0usize;

        for cookie in free_cookies {
            let Some(worker) = self.select_worker_for_cookie(&workers, &worker_assignments, cookie.domain.as_str()) else {
                skipped_by_domain_constraint += 1;
                continue;
            };

            let Some(data) = self.read_cookie_data(cookie, &mut read_failed_count) else {
                continue;
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
            self.assignments.insert(cookie.cookie_id.clone(), worker_id.clone());
            worker_assignments.register(&worker_id, cookie.domain.as_str());
            assigned_count += 1;
            info!(node = %worker.address, cookie_id = %cookie.cookie_id, domain = %cookie.domain, "Cookie assigned and pushed");
        }

        if assigned_count > 0 || read_failed_count > 0 || push_failed_count > 0 || skipped_by_domain_constraint > 0 {
            info!(
                assigned_count,
                read_failed_count, push_failed_count, skipped_by_domain_constraint, "Cookie assignment free cookies processed"
            );
        }
    }

    async fn revoke_removed_source_cookies(
        &mut self,
        cookies: &[CookieRecord],
        available_workers: &HashMap<String, AssignmentNodeHandle>,
        worker_cookie_ids: &HashMap<String, HashSet<String>>,
    ) {
        let source_cookie_ids = cookies.iter().map(|cookie| cookie.cookie_id.as_str()).collect::<HashSet<_>>();
        let removed_assignments = self
            .assignments
            .iter()
            .filter(|(cookie_id, _)| !source_cookie_ids.contains(cookie_id.as_str()))
            .map(|(cookie_id, worker_id)| (cookie_id.clone(), worker_id.clone()))
            .collect::<Vec<_>>();

        let mut removable_cookie_ids = HashSet::new();
        let mut revoked_count = 0usize;
        let mut revoke_failed_count = 0usize;
        let mut pending_unavailable_count = 0usize;

        for (cookie_id, worker_id) in removed_assignments {
            if available_workers.contains_key(&worker_id) && !worker_cookie_ids.contains_key(&worker_id) {
                pending_unavailable_count += 1;
                continue;
            }

            let Some(worker) = available_workers.get(&worker_id) else {
                removable_cookie_ids.insert(cookie_id);
                pending_unavailable_count += 1;
                continue;
            };

            match worker.remove_cookie(&cookie_id).await {
                Ok(()) => {
                    removable_cookie_ids.insert(cookie_id);
                    revoked_count += 1;
                }
                Err(err) => {
                    revoke_failed_count += 1;
                    warn!(node = %worker.address, cookie_id = %cookie_id, error = %err, "Failed to revoke removed source cookie from worker");
                }
            }
        }

        self.assignments.retain(|cookie_id, _| !removable_cookie_ids.contains(cookie_id));

        if revoked_count > 0 || revoke_failed_count > 0 || pending_unavailable_count > 0 {
            info!(
                revoked_count,
                revoke_failed_count, pending_unavailable_count, "Removed source cookies reconciled"
            );
        }
    }

    fn load_source_cookies(&self) -> Option<Vec<CookieRecord>> {
        match load_cookies_from_directory(Path::new(COOKIE_DIR)) {
            Ok(cookies) => Some(cookies),
            Err(err) => {
                warn!(cookie_dir = COOKIE_DIR, error = %err, "Failed to load cookie files");
                None
            }
        }
    }

    fn cookie_domains<'a>(&self, cookies: &'a [CookieRecord]) -> HashMap<&'a str, &'a str> {
        cookies
            .iter()
            .map(|cookie| (cookie.cookie_id.as_str(), cookie.domain.as_str()))
            .collect()
    }

    fn build_worker_assignments<'a>(&self, cookie_domains: &HashMap<&'a str, &'a str>) -> WorkerAssignments<'a> {
        let mut worker_assignments = WorkerAssignments::default();

        for (cookie_id, worker_id) in &self.assignments {
            if let Some(&domain) = cookie_domains.get(cookie_id.as_str()) {
                worker_assignments.register(worker_id, domain);
            }
        }

        worker_assignments
    }

    fn select_worker_for_cookie<'a>(
        &self,
        workers: &[&'a AssignmentNodeHandle],
        worker_assignments: &WorkerAssignments<'_>,
        domain: &str,
    ) -> Option<&'a AssignmentNodeHandle> {
        workers
            .iter()
            .copied()
            .filter(|worker| !worker_assignments.has_domain(worker.address.as_ref(), domain))
            .min_by(|left, right| self.compare_workers(worker_assignments, left, right))
    }

    fn compare_workers(
        &self,
        worker_assignments: &WorkerAssignments<'_>,
        left: &AssignmentNodeHandle,
        right: &AssignmentNodeHandle,
    ) -> std::cmp::Ordering {
        worker_assignments
            .cookie_count(left.address.as_ref())
            .cmp(&worker_assignments.cookie_count(right.address.as_ref()))
            .then_with(|| {
                worker_assignments
                    .domain_count(left.address.as_ref())
                    .cmp(&worker_assignments.domain_count(right.address.as_ref()))
            })
            .then_with(|| left.address.cmp(&right.address))
    }

    fn read_cookie_data(&self, cookie: &CookieRecord, read_failed_count: &mut usize) -> Option<String> {
        match fs::read_to_string(&cookie.path) {
            Ok(data) => Some(data),
            Err(err) => {
                *read_failed_count += 1;
                warn!(cookie_id = %cookie.cookie_id, path = %cookie.path.display(), error = %err, "Failed to read cookie file");
                None
            }
        }
    }
}

impl<'a> WorkerAssignments<'a> {
    fn register(&mut self, worker_id: &str, domain: &'a str) {
        self.domains.entry(worker_id.to_owned()).or_default().insert(domain);
        *self.cookie_counts.entry(worker_id.to_owned()).or_default() += 1;
    }

    fn has_domain(&self, worker_id: &str, domain: &str) -> bool {
        self.domains.get(worker_id).is_some_and(|domains| domains.contains(domain))
    }

    fn cookie_count(&self, worker_id: &str) -> usize {
        self.cookie_counts.get(worker_id).copied().unwrap_or_default()
    }

    fn domain_count(&self, worker_id: &str) -> usize {
        self.domains.get(worker_id).map_or(0, HashSet::len)
    }
}
