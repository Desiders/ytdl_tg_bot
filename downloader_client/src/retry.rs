use std::{collections::HashSet, fmt::Display, future::Future, sync::Arc};
use tracing::{error, warn};

use crate::{NodeHandle, NodeRouter};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeAttemptErrorKind {
    ResourceExhausted,
    Unavailable,
    ExecutionUncertain,
    ContextUnavailable,
    Unauthenticated,
    Fatal,
}

#[derive(Debug)]
pub enum NodeFailoverError<E> {
    AllNodesBusy,
    NodeUnavailable,
    NodeContextUnavailable,
    ExecutionUncertain,
    Operation(E),
}

/// Executes an operation against downloader nodes with router-based failover.
///
/// # Errors
///
/// Returns [`NodeFailoverError`] if no suitable node is available, all retryable
/// attempts fail, or the operation returns a fatal error.
pub async fn with_node_failover<T, E, F, Fut, C>(
    router: &NodeRouter,
    domain: Option<&str>,
    mut execute: F,
    classify_error: C,
) -> Result<T, NodeFailoverError<E>>
where
    E: Display,
    F: FnMut(Arc<NodeHandle>) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    C: Fn(&E) -> NodeAttemptErrorKind,
{
    let mut excluded = HashSet::new();
    let mut saw_retryable_context_error = false;
    let mut saw_busy = false;

    loop {
        let Some(node) = router.pick_node(domain, &excluded) else {
            return if saw_busy {
                Err(NodeFailoverError::AllNodesBusy)
            } else if saw_retryable_context_error {
                Err(NodeFailoverError::NodeContextUnavailable)
            } else {
                Err(NodeFailoverError::NodeUnavailable)
            };
        };

        let result = execute(node.clone()).await;

        match result {
            Ok(result) => return Ok(result),
            Err(err) => match classify_error(&err) {
                NodeAttemptErrorKind::ResourceExhausted => {
                    saw_busy = true;
                    excluded.insert(node.address.to_string());
                }
                NodeAttemptErrorKind::ContextUnavailable => {
                    warn!(node = %node.address, error = %err, "Download node returned retryable source-context error");
                    saw_retryable_context_error = true;
                    excluded.insert(node.address.to_string());
                }
                NodeAttemptErrorKind::Unavailable => {
                    node.mark_unavailable();
                    warn!(node = %node.address, error = %err, "Download node unavailable");
                    excluded.insert(node.address.to_string());
                }
                NodeAttemptErrorKind::ExecutionUncertain => {
                    node.mark_unavailable();
                    warn!(node = %node.address, error = %err, "Download execution outcome is uncertain");
                    return Err(NodeFailoverError::ExecutionUncertain);
                }
                NodeAttemptErrorKind::Unauthenticated => {
                    error!(node = %node.address, error = %err, "Download node authentication failed");
                    return Err(NodeFailoverError::NodeUnavailable);
                }
                NodeAttemptErrorKind::Fatal => {
                    return Err(NodeFailoverError::Operation(err));
                }
            },
        }
    }
}
