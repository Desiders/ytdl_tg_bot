use bytes::Bytes;
use proto::downloader::{download_chunk::Payload, downloader_client::DownloaderClient, DownloadChunk, DownloadMeta, DownloadRequest};
use tonic::Code;

use crate::{
    authenticated_request,
    outcome::{current_execution_outcome, mark_execution_uncertain, ExecutionOutcome},
    retry::{with_node_failover, NodeFailoverError},
    DownloadErrorKind, NodeAttemptErrorKind, NodeRouter,
};

pub enum DownloadEvent {
    Progress(String),
    Data(Bytes),
    ThumbnailData(Bytes),
}

pub struct DownloadSession {
    meta: DownloadMeta,
    stream: tonic::Streaming<DownloadChunk>,
    outcome: Option<ExecutionOutcome>,
}

impl DownloadSession {
    #[must_use]
    pub fn meta(&self) -> &DownloadMeta {
        &self.meta
    }

    /// Reads the next event from the downloader stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the stream RPC fails or the downloader sends an
    /// invalid chunk sequence.
    pub async fn next_event(&mut self) -> Result<Option<DownloadEvent>, DownloadErrorKind> {
        let chunk = match self.stream.message().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => return Ok(None),
            Err(status) => {
                let error = stream_error(status);
                if error.is_execution_uncertain() {
                    if let Some(outcome) = &self.outcome {
                        outcome.mark_uncertain();
                    }
                }
                return Err(error);
            }
        };

        match chunk.payload {
            Some(Payload::Progress(progress)) => Ok(Some(DownloadEvent::Progress(progress))),
            Some(Payload::Data(data)) => Ok(Some(DownloadEvent::Data(Bytes::from(data)))),
            Some(Payload::ThumbnailData(data)) => Ok(Some(DownloadEvent::ThumbnailData(Bytes::from(data)))),
            Some(Payload::Meta(_)) | None => {
                if let Some(outcome) = &self.outcome {
                    outcome.mark_uncertain();
                }
                Err(DownloadErrorKind::ExecutionUncertain)
            }
        }
    }
}

/// Starts media download on a routed downloader node.
///
/// The node streams `Progress` chunks while the download runs and only sends `Meta`
/// once the download has succeeded and its output file is validated. This function
/// forwards those pre-`Meta` progress updates through `on_progress` and returns the
/// session at `Meta` — so a download failure surfaces here as an error (and can fail
/// over / be retried with another format) instead of corrupting an already-started upload.
///
/// # Errors
///
/// Returns an error if no node is available, authentication metadata cannot be
/// built, the RPC fails, or the stream sends an invalid chunk sequence.
pub async fn download_media(
    router: &NodeRouter,
    domain: Option<&str>,
    request: DownloadRequest,
    on_progress: impl Fn(String) + Sync,
) -> Result<DownloadSession, DownloadErrorKind> {
    let on_progress = &on_progress;
    let result = with_node_failover(
        router,
        domain,
        |node| {
            let request = request.clone();
            async move {
                let mut client = DownloaderClient::new(node.channel.clone());
                let response = client
                    .download_media(authenticated_request(request, &node.token)?)
                    .await
                    .map_err(start_error)?;
                if let Some(outcome) = current_execution_outcome() {
                    outcome.mark_execution_started();
                }
                let mut stream = response.into_inner();

                loop {
                    let chunk = stream
                        .message()
                        .await
                        .map_err(stream_error)?
                        .ok_or(DownloadErrorKind::InvalidStream)?;
                    let Some(payload) = chunk.payload else {
                        mark_execution_uncertain();
                        return Err(DownloadErrorKind::ExecutionUncertain);
                    };
                    match payload {
                        Payload::Progress(progress) => on_progress(progress),
                        Payload::Meta(meta) => {
                            return Ok::<_, DownloadErrorKind>(DownloadSession {
                                meta,
                                stream,
                                outcome: current_execution_outcome(),
                            });
                        }
                        Payload::Data(_) | Payload::ThumbnailData(_) => {
                            mark_execution_uncertain();
                            return Err(DownloadErrorKind::ExecutionUncertain);
                        }
                    }
                }
            }
        },
        classify_download_error,
    )
    .await;
    map_download_result(result)
}

fn map_download_result(
    result: Result<DownloadSession, NodeFailoverError<DownloadErrorKind>>,
) -> Result<DownloadSession, DownloadErrorKind> {
    if matches!(&result, Err(NodeFailoverError::AllNodesBusy)) {
        if let Some(outcome) = current_execution_outcome() {
            outcome.mark_all_nodes_busy();
        }
    }
    result.map_err(DownloadErrorKind::from)
}

fn start_error(status: tonic::Status) -> DownloadErrorKind {
    match status.code() {
        // The request may have reached the node before the connection failed.
        Code::Unavailable | Code::Unknown | Code::Cancelled | Code::DeadlineExceeded => {
            mark_execution_uncertain();
            DownloadErrorKind::ExecutionUncertain
        }
        _ => DownloadErrorKind::Rpc(status),
    }
}

fn stream_error(status: tonic::Status) -> DownloadErrorKind {
    match status.code() {
        // Once a stream exists, transport failure provides no proof the task stopped.
        Code::Unavailable | Code::Unknown | Code::Cancelled | Code::DeadlineExceeded => {
            mark_execution_uncertain();
            DownloadErrorKind::ExecutionUncertain
        }
        _ => DownloadErrorKind::Rpc(status),
    }
}

fn classify_download_error(err: &DownloadErrorKind) -> NodeAttemptErrorKind {
    match err {
        DownloadErrorKind::Rpc(status) if status.code() == Code::ResourceExhausted => NodeAttemptErrorKind::ResourceExhausted,
        DownloadErrorKind::Rpc(status) if status.code() == Code::Aborted => NodeAttemptErrorKind::ContextUnavailable,
        DownloadErrorKind::Rpc(status) if status.code() == Code::Unavailable => NodeAttemptErrorKind::Unavailable,
        DownloadErrorKind::ExecutionUncertain => NodeAttemptErrorKind::ExecutionUncertain,
        DownloadErrorKind::Rpc(status) if status.code() == Code::Unauthenticated => NodeAttemptErrorKind::Unauthenticated,
        _ => NodeAttemptErrorKind::Fatal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::track_execution;

    #[test]
    fn transport_failure_does_not_permit_cross_node_execution() {
        let error = start_error(tonic::Status::unavailable("synthetic transport loss"));
        assert!(matches!(error, DownloadErrorKind::ExecutionUncertain));
        assert_eq!(classify_download_error(&error), NodeAttemptErrorKind::ExecutionUncertain);
    }

    #[tokio::test]
    async fn uncertain_start_result_is_retained_by_the_worker_outcome() {
        let (error, outcome) = track_execution(async { start_error(tonic::Status::unavailable("synthetic transport loss")) }).await;
        assert!(error.is_execution_uncertain());
        assert!(outcome.is_uncertain());
    }

    #[test]
    fn initial_capacity_rejection_remains_safe_to_fail_over() {
        let error = start_error(tonic::Status::resource_exhausted("synthetic full node"));
        assert_eq!(classify_download_error(&error), NodeAttemptErrorKind::ResourceExhausted);
    }

    #[test]
    fn deadline_after_stream_start_is_an_uncertain_execution() {
        let error = stream_error(tonic::Status::deadline_exceeded("synthetic timeout"));
        assert!(error.is_execution_uncertain());
        assert_eq!(classify_download_error(&error), NodeAttemptErrorKind::ExecutionUncertain);
    }

    #[tokio::test]
    async fn all_nodes_rejecting_admission_is_reported_without_consuming_an_attempt() {
        let (result, outcome) = track_execution(async { map_download_result(Err(NodeFailoverError::AllNodesBusy)) }).await;

        assert!(matches!(result, Err(DownloadErrorKind::NodeUnavailable)));
        assert!(outcome.all_nodes_busy());
        assert!(!outcome.is_uncertain());
    }
}
