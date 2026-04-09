use std::{collections::HashSet, fmt::Display, future::Future, sync::Arc};
use tracing::{error, warn};

use crate::{NodeHandle, NodeRouter};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeAttemptErrorKind {
    ResourceExhausted,
    Unavailable,
    ContextUnavailable,
    Unauthenticated,
    Fatal,
}

#[derive(Debug)]
pub enum NodeFailoverError<E> {
    NodeUnavailable,
    NodeContextUnavailable,
    Operation(E),
}

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

    loop {
        let Some(node) = router.pick_node(domain, &excluded) else {
            return if saw_retryable_context_error {
                Err(NodeFailoverError::NodeContextUnavailable)
            } else {
                Err(NodeFailoverError::NodeUnavailable)
            };
        };

        node.reserve_download_slot();
        let result = execute(node.clone()).await;
        node.release_download_slot();

        match result {
            Ok(result) => return Ok(result),
            Err(err) => match classify_error(&err) {
                NodeAttemptErrorKind::ResourceExhausted => {
                    excluded.insert(node.address.to_string());
                }
                NodeAttemptErrorKind::ContextUnavailable => {
                    warn!(node = %node.address, error = %err, "Download node returned retryable source-context error");
                    saw_retryable_context_error = true;
                    excluded.insert(node.address.to_string());
                }
                NodeAttemptErrorKind::Unavailable => {
                    warn!(node = %node.address, error = %err, "Download node unavailable");
                    excluded.insert(node.address.to_string());
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
