use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

tokio::task_local! {
    static EXECUTION_OUTCOME: ExecutionOutcome;
}

/// Tracks downloader execution state that must survive framework error erasure.
///
/// A clone is carried by the post-`Meta` stream forwarder, which runs in a
/// separate Tokio task from the worker operation.
#[derive(Clone, Default)]
pub struct ExecutionOutcome {
    uncertain: Arc<AtomicBool>,
    all_nodes_busy: Arc<AtomicBool>,
    execution_started: Arc<AtomicBool>,
}

impl ExecutionOutcome {
    pub fn mark_uncertain(&self) {
        self.uncertain.store(true, Ordering::Relaxed);
    }

    /// Records that every candidate rejected admission before starting execution.
    pub fn mark_all_nodes_busy(&self) {
        self.all_nodes_busy.store(true, Ordering::Relaxed);
    }

    /// Records that a downloader accepted a request after acquiring node capacity.
    pub fn mark_execution_started(&self) {
        self.execution_started.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_uncertain(&self) -> bool {
        self.uncertain.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn all_nodes_busy(&self) -> bool {
        self.all_nodes_busy.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn execution_started(&self) -> bool {
        self.execution_started.load(Ordering::Relaxed)
    }
}

/// Runs a worker operation while retaining whether a downloader RPC became
/// ambiguous after it may have started remote execution.
pub async fn track_execution<T>(future: impl Future<Output = T>) -> (T, ExecutionOutcome) {
    if let Ok(outcome) = EXECUTION_OUTCOME.try_with(Clone::clone) {
        return (future.await, outcome);
    }

    let outcome = ExecutionOutcome::default();
    let result = EXECUTION_OUTCOME
        .scope(outcome.clone(), async {
            let result = future.await;
            result
        })
        .await;
    (result, outcome)
}

pub(crate) fn mark_execution_uncertain() {
    let _ = EXECUTION_OUTCOME.try_with(|outcome| outcome.mark_uncertain());
}

pub(crate) fn current_execution_outcome() -> Option<ExecutionOutcome> {
    EXECUTION_OUTCOME.try_with(Clone::clone).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn outcome_is_shared_with_a_spawned_stream_task() {
        let ((), outcome) = track_execution(async {
            let outcome = current_execution_outcome().unwrap();
            tokio::spawn(async move { outcome.mark_uncertain() }).await.unwrap();
        })
        .await;
        assert!(outcome.is_uncertain());
    }

    #[tokio::test]
    async fn nested_tracking_keeps_the_outer_outcome() {
        let ((), outcome) = track_execution(async {
            mark_execution_uncertain();
            let ((), nested_outcome) = track_execution(async {
                assert!(current_execution_outcome().unwrap().is_uncertain());
            })
            .await;

            assert!(nested_outcome.is_uncertain());
            assert!(current_execution_outcome().unwrap().is_uncertain());
        })
        .await;

        assert!(outcome.is_uncertain());
    }

    #[tokio::test]
    async fn concurrent_executions_have_independent_outcomes() {
        let (first, second) = tokio::join!(
            track_execution(async {
                mark_execution_uncertain();
                tokio::task::yield_now().await;
                current_execution_outcome().unwrap().is_uncertain()
            }),
            track_execution(async {
                tokio::task::yield_now().await;
                current_execution_outcome().unwrap().is_uncertain()
            }),
        );

        assert!(first.0);
        assert!(first.1.is_uncertain());
        assert!(!second.0);
        assert!(!second.1.is_uncertain());
    }

    #[tokio::test]
    async fn each_top_level_execution_starts_with_a_fresh_outcome() {
        let (_, first) = track_execution(async {
            mark_execution_uncertain();
        })
        .await;
        let (_, second) = track_execution(async {
            assert!(!current_execution_outcome().unwrap().is_uncertain());
            assert!(!current_execution_outcome().unwrap().all_nodes_busy());
            assert!(!current_execution_outcome().unwrap().execution_started());
        })
        .await;

        assert!(first.is_uncertain());
        assert!(!second.is_uncertain());
        assert!(!second.all_nodes_busy());
    }

    #[tokio::test]
    async fn admission_pressure_is_monotonic_and_isolated_per_execution() {
        let (first, second) = tokio::join!(
            track_execution(async {
                let outcome = current_execution_outcome().unwrap();
                outcome.mark_all_nodes_busy();
                tokio::task::yield_now().await;
                outcome.all_nodes_busy()
            }),
            track_execution(async {
                tokio::task::yield_now().await;
                current_execution_outcome().unwrap().all_nodes_busy()
            }),
        );

        assert!(first.0);
        assert!(first.1.all_nodes_busy());
        assert!(!first.1.execution_started());
        assert!(!second.0);
        assert!(!second.1.all_nodes_busy());
    }

    #[tokio::test]
    async fn a_later_admitted_format_means_the_job_consumed_an_execution_opportunity() {
        let (_, outcome) = track_execution(async {
            let outcome = current_execution_outcome().unwrap();
            outcome.mark_all_nodes_busy();
            outcome.mark_execution_started();
        })
        .await;

        assert!(outcome.all_nodes_busy());
        assert!(outcome.execution_started());
    }
}
