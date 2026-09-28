use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{sync::watch, time::Instant};

tokio::task_local! {
    static EXECUTION_OUTCOME: ExecutionOutcome;
}

/// Tracks downloader execution state that must survive framework error erasure.
///
/// A clone is carried by the post-`Meta` stream forwarder, which runs in a
/// separate Tokio task from the worker operation.
#[derive(Clone)]
pub struct ExecutionOutcome {
    state: Arc<AtomicBool>,
    progress: watch::Sender<Instant>,
}

impl Default for ExecutionOutcome {
    fn default() -> Self {
        Self {
            state: Arc::new(AtomicBool::new(false)),
            progress: watch::channel(Instant::now()).0,
        }
    }
}

impl ExecutionOutcome {
    pub fn record_progress(&self) {
        self.progress.send_replace(Instant::now());
    }

    #[must_use]
    pub fn subscribe_progress(&self) -> watch::Receiver<Instant> {
        self.progress.subscribe()
    }

    pub async fn scope<T>(&self, future: impl Future<Output = T>) -> T {
        EXECUTION_OUTCOME.scope(self.clone(), future).await
    }

    pub fn mark_uncertain(&self) {
        self.state.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_uncertain(&self) -> bool {
        self.state.load(Ordering::Relaxed)
    }
}

pub fn record_execution_progress() {
    let _ = EXECUTION_OUTCOME.try_with(ExecutionOutcome::record_progress);
}

/// Runs a worker operation while retaining whether a downloader RPC became
/// ambiguous after it may have started remote execution.
pub async fn track_execution<T>(future: impl Future<Output = T>) -> (T, ExecutionOutcome) {
    if let Ok(outcome) = EXECUTION_OUTCOME.try_with(Clone::clone) {
        return (future.await, outcome);
    }

    let outcome = ExecutionOutcome::default();
    let result = EXECUTION_OUTCOME.scope(outcome.clone(), future).await;
    (result, outcome)
}

pub(crate) fn mark_execution_uncertain() {
    let _ = EXECUTION_OUTCOME.try_with(ExecutionOutcome::mark_uncertain);
}

#[must_use]
pub fn current_execution_outcome() -> Option<ExecutionOutcome> {
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
        let ((), first) = track_execution(async {
            mark_execution_uncertain();
        })
        .await;
        let ((), second) = track_execution(async {
            assert!(!current_execution_outcome().unwrap().is_uncertain());
        })
        .await;

        assert!(first.is_uncertain());
        assert!(!second.is_uncertain());
    }
}
