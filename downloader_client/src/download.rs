use bytes::Bytes;
use proto::downloader::{download_chunk::Payload, downloader_client::DownloaderClient, DownloadChunk, DownloadMeta, DownloadRequest};
use tokio::time::Instant;
use tonic::Code;
use tracing::{debug, info};

use crate::{
    authenticated_request,
    outcome::{current_execution_outcome, mark_execution_uncertain, record_execution_progress, ExecutionOutcome},
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
    complete: bool,
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
        if self.complete {
            return Ok(None);
        }
        loop {
            let chunk = match self.stream.message().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => return Err(self.invalid_stream()),
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
                Some(Payload::Progress(progress)) => {
                    if let Some(outcome) = &self.outcome {
                        outcome.record_progress();
                    }
                    return Ok(Some(DownloadEvent::Progress(progress)));
                }
                Some(Payload::Data(data)) => {
                    if !data.is_empty() {
                        if let Some(outcome) = &self.outcome {
                            outcome.record_progress();
                        }
                    }
                    return Ok(Some(DownloadEvent::Data(Bytes::from(data))));
                }
                Some(Payload::ThumbnailData(data)) => return Ok(Some(DownloadEvent::ThumbnailData(Bytes::from(data)))),
                Some(Payload::Complete(true)) => {
                    self.complete = true;
                    return Ok(None);
                }
                Some(Payload::Complete(false) | Payload::Meta(_)) | None => return Err(self.invalid_stream()),
            }
        }
    }

    fn invalid_stream(&self) -> DownloadErrorKind {
        if let Some(outcome) = &self.outcome {
            outcome.mark_uncertain();
        }
        DownloadErrorKind::InvalidStream
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
    deadline: &mut Instant,
) -> Result<DownloadSession, DownloadErrorKind> {
    let on_progress = &on_progress;
    let mut capacity = router.subscribe_capacity();
    let mut waiting_for_capacity = false;
    loop {
        capacity.borrow_and_update();
        let attempt_deadline = *deadline;
        let result = tokio::time::timeout_at(
            attempt_deadline,
            with_node_failover(
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
                        let mut stream = response.into_inner();

                        loop {
                            let chunk = stream.message().await.map_err(stream_error)?;
                            let Some(chunk) = chunk else {
                                mark_execution_uncertain();
                                return Err(DownloadErrorKind::InvalidStream);
                            };
                            let Some(payload) = chunk.payload else {
                                mark_execution_uncertain();
                                return Err(DownloadErrorKind::InvalidStream);
                            };
                            match payload {
                                Payload::Progress(progress) => {
                                    record_execution_progress();
                                    on_progress(progress);
                                }
                                Payload::Meta(meta) => {
                                    record_execution_progress();
                                    return Ok::<_, DownloadErrorKind>(DownloadSession {
                                        meta,
                                        stream,
                                        outcome: current_execution_outcome(),
                                        complete: false,
                                    });
                                }
                                Payload::Data(_) | Payload::ThumbnailData(_) | Payload::Complete(_) => {
                                    mark_execution_uncertain();
                                    return Err(DownloadErrorKind::InvalidStream);
                                }
                            }
                        }
                    }
                },
                classify_download_error,
            ),
        )
        .await;
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                mark_execution_uncertain();
                return Err(DownloadErrorKind::MediaTimeout);
            }
        };
        if matches!(result, Err(NodeFailoverError::AllNodesBusy | NodeFailoverError::NodeUnavailable)) {
            // No node admitted this request. Keep the same operation/delivery and
            // reroute only after another status sample, rather than reporting a failure.
            if !waiting_for_capacity {
                info!("No downloader node admitted media; waiting for a status change");
                waiting_for_capacity = true;
            }
            let waiting_since = Instant::now();
            if !NodeRouter::wait_for_status_change(&mut capacity).await {
                return Err(DownloadErrorKind::NodeUnavailable);
            }
            *deadline += waiting_since.elapsed();
            continue;
        }
        if waiting_for_capacity {
            debug!("Downloader admission wait ended");
        }
        return result.map_err(DownloadErrorKind::from);
    }
}

fn start_error(status: tonic::Status) -> DownloadErrorKind {
    match status.code() {
        // The request may have reached the node before the connection failed.
        Code::Unavailable | Code::Unknown | Code::Cancelled | Code::DeadlineExceeded | Code::Internal => {
            mark_execution_uncertain();
            DownloadErrorKind::ExecutionUncertain
        }
        Code::ResourceExhausted if status.message() != proto::NODE_CAPACITY_REJECTION_MESSAGE => {
            mark_execution_uncertain();
            DownloadErrorKind::ExecutionUncertain
        }
        _ => DownloadErrorKind::Rpc(status),
    }
}

fn stream_error(status: tonic::Status) -> DownloadErrorKind {
    if status
        .metadata()
        .get(proto::TERMINAL_DOWNLOAD_STATUS_HEADER)
        .is_some_and(|value| value == "true")
    {
        DownloadErrorKind::Rpc(status)
    } else {
        // Tonic can also produce INTERNAL for an HTTP/2 failure; the code alone
        // does not prove that a prior remote execution has ended.
        mark_execution_uncertain();
        DownloadErrorKind::ExecutionUncertain
    }
}

fn classify_download_error(err: &DownloadErrorKind) -> NodeAttemptErrorKind {
    match err {
        DownloadErrorKind::Rpc(status)
            if status.code() == Code::ResourceExhausted
                && status.message() == proto::NODE_CAPACITY_REJECTION_MESSAGE
                && !status.metadata().contains_key(proto::TERMINAL_DOWNLOAD_STATUS_HEADER) =>
        {
            NodeAttemptErrorKind::ResourceExhausted
        }
        DownloadErrorKind::Rpc(status) if status.code() == Code::Aborted => NodeAttemptErrorKind::ContextUnavailable,
        DownloadErrorKind::Rpc(status) if status.code() == Code::Unavailable => NodeAttemptErrorKind::Unavailable,
        DownloadErrorKind::ExecutionUncertain | DownloadErrorKind::InvalidStream | DownloadErrorKind::MediaTimeout => {
            NodeAttemptErrorKind::ExecutionUncertain
        }
        _ => NodeAttemptErrorKind::Fatal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::track_execution;
    use futures_util::{stream, Stream};
    use proto::downloader::{
        downloader_server::{Downloader, DownloaderServer},
        MediaInfoRequest, MediaInfoResponse,
    };
    use std::{
        pin::Pin,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };
    use tonic::{metadata::MetadataValue, Request, Response, Status};

    #[derive(Clone, Default)]
    struct FakeDownloader {
        busy: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
        unavailable: bool,
        aborted: bool,
        stream_status: Option<Status>,
        missing_completion: bool,
        error_after_completion: bool,
        stall_before_meta: bool,
        invalid_first_chunk: bool,
    }

    #[tonic::async_trait]
    impl Downloader for FakeDownloader {
        type DownloadMediaStream = Pin<Box<dyn Stream<Item = Result<DownloadChunk, Status>> + Send>>;

        async fn get_media_info(&self, _: Request<MediaInfoRequest>) -> Result<Response<MediaInfoResponse>, Status> {
            Err(Status::unimplemented("Synthetic metadata"))
        }

        async fn download_media(&self, _: Request<DownloadRequest>) -> Result<Response<Self::DownloadMediaStream>, Status> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.unavailable {
                return Err(Status::unavailable("Synthetic lost transport"));
            }
            if self.aborted {
                return Err(Status::aborted("Synthetic completed source rejection"));
            }
            if self.busy.load(Ordering::Relaxed) {
                return Err(Status::resource_exhausted("Node is at capacity"));
            }
            if let Some(status) = &self.stream_status {
                return Ok(Response::new(Box::pin(stream::iter([Err(status.clone())]))));
            }
            if self.stall_before_meta {
                return Ok(Response::new(Box::pin(stream::pending::<Result<DownloadChunk, Status>>())));
            }
            if self.invalid_first_chunk {
                return Ok(Response::new(Box::pin(stream::iter([Ok(DownloadChunk {
                    payload: Some(Payload::Data(vec![42])),
                })]))));
            }
            let mut chunks = vec![
                Ok(DownloadChunk {
                    payload: Some(Payload::Meta(DownloadMeta::default())),
                }),
                Ok(DownloadChunk {
                    payload: Some(Payload::Data(vec![42])),
                }),
            ];
            if !self.missing_completion {
                chunks.push(Ok(DownloadChunk {
                    payload: Some(Payload::Complete(true)),
                }));
            }
            if self.error_after_completion {
                chunks.push(Err(Status::internal("Synthetic connection error after completion")));
            }
            Ok(Response::new(Box::pin(stream::iter(chunks))))
        }
    }

    async fn fake_node(service: FakeDownloader) -> (Arc<crate::NodeHandle>, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let incoming = Box::pin(stream::unfold(listener, |listener| async {
            Some((listener.accept().await.map(|(socket, _)| socket), listener))
        }));
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(DownloaderServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        let node = Arc::new(crate::NodeHandle::new(
            "test".into(),
            address.clone().into(),
            "test".into(),
            tonic::transport::Endpoint::from_shared(address).unwrap().connect_lazy(),
        ));
        node.update_remote_status(0, 1);
        (node, task)
    }

    #[tokio::test]
    async fn all_busy_nodes_wait_for_a_fresh_status_before_rerouting_the_same_request() {
        let service = FakeDownloader::default();
        service.busy.store(true, Ordering::Relaxed);
        let (first, first_task) = fake_node(service.clone()).await;
        let (second, second_task) = fake_node(service.clone()).await;
        let (router, capacity) = NodeRouter::with_test_nodes(vec![first, second]);
        let ((), outcome) = track_execution(async {
            let mut deadline = Instant::now() + Duration::from_secs(5);
            let download = download_media(&router, None, DownloadRequest::default(), |_| {}, &mut deadline);
            tokio::pin!(download);
            assert!(tokio::time::timeout(Duration::from_secs(1), &mut download).await.is_err());
            assert_eq!(service.calls.load(Ordering::Relaxed), 2);
            service.busy.store(false, Ordering::Relaxed);
            capacity.send_replace(crate::Capacity { available: 2, total: 2 });
            let mut session = tokio::time::timeout(Duration::from_secs(3), download).await.unwrap().unwrap();
            assert!(matches!(session.next_event().await.unwrap(), Some(DownloadEvent::Data(data)) if data == [42][..]));
            assert_eq!(service.calls.load(Ordering::Relaxed), 3);
        })
        .await;
        assert!(!outcome.is_uncertain());
        first_task.abort();
        second_task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn unavailable_node_waits_without_spending_the_execution_deadline() {
        let service = FakeDownloader::default();
        let (node, server) = fake_node(service.clone()).await;
        node.mark_unavailable();
        let (router, capacity) = NodeRouter::with_test_nodes(vec![node.clone()]);
        let mut deadline = Instant::now() + Duration::from_secs(1);
        let download = download_media(&router, None, DownloadRequest::default(), |_| {}, &mut deadline);
        tokio::pin!(download);
        tokio::select! {
            _ = &mut download => panic!("Unavailable node unexpectedly completed"),
            () = tokio::time::sleep(Duration::from_millis(1)) => {}
        }
        assert_eq!(service.calls.load(Ordering::Relaxed), 0);

        tokio::time::advance(Duration::from_secs(3)).await;
        node.update_remote_status(0, 1);
        capacity.send_replace(crate::Capacity { available: 1, total: 1 });
        let mut session = download.await.unwrap();
        assert!(matches!(session.next_event().await.unwrap(), Some(DownloadEvent::Data(_))));
        assert_eq!(service.calls.load(Ordering::Relaxed), 1);
        server.abort();
    }

    #[tokio::test]
    async fn ambiguous_dispatch_never_calls_a_second_node() {
        let service = FakeDownloader {
            unavailable: true,
            ..FakeDownloader::default()
        };
        let (first, first_task) = fake_node(service.clone()).await;
        let (second, second_task) = fake_node(service.clone()).await;
        let (router, _) = NodeRouter::with_test_nodes(vec![first, second]);
        let mut deadline = Instant::now() + Duration::from_secs(5);
        let (result, outcome) = track_execution(download_media(&router, None, DownloadRequest::default(), |_| {}, &mut deadline)).await;
        assert!(matches!(result, Err(DownloadErrorKind::ExecutionUncertain)));
        assert!(outcome.is_uncertain());
        assert_eq!(service.calls.load(Ordering::Relaxed), 1);
        first_task.abort();
        second_task.abort();
    }

    #[tokio::test]
    async fn known_ended_source_failure_can_try_another_node() {
        let rejected = FakeDownloader {
            aborted: true,
            ..FakeDownloader::default()
        };
        let accepted = FakeDownloader::default();
        let (first, first_task) = fake_node(rejected.clone()).await;
        let (second, second_task) = fake_node(accepted.clone()).await;
        // The larger advertised capacity makes the rejecting node the deterministic first choice.
        first.update_remote_status(0, 2);
        let (router, _) = NodeRouter::with_test_nodes(vec![first, second]);
        let mut deadline = Instant::now() + Duration::from_secs(5);
        let session = download_media(&router, None, DownloadRequest::default(), |_| {}, &mut deadline)
            .await
            .unwrap();
        assert_eq!(session.meta(), &DownloadMeta::default());
        assert_eq!(rejected.calls.load(Ordering::Relaxed), 1);
        assert_eq!(accepted.calls.load(Ordering::Relaxed), 1);
        first_task.abort();
        second_task.abort();
    }

    #[tokio::test]
    async fn local_media_deadline_reports_timeout_and_stops_node_failover() {
        let stalled = FakeDownloader {
            stall_before_meta: true,
            ..FakeDownloader::default()
        };
        let other = FakeDownloader::default();
        let (first, first_task) = fake_node(stalled.clone()).await;
        let (second, second_task) = fake_node(other.clone()).await;
        first.update_remote_status(0, 2);
        let (router, _) = NodeRouter::with_test_nodes(vec![first, second]);
        let mut deadline = Instant::now() + Duration::from_secs(2);
        let (result, outcome) = track_execution(download_media(&router, None, DownloadRequest::default(), |_| {}, &mut deadline)).await;
        assert!(matches!(result, Err(DownloadErrorKind::MediaTimeout)));
        assert!(outcome.is_uncertain());
        assert_eq!(stalled.calls.load(Ordering::Relaxed), 1);
        assert_eq!(other.calls.load(Ordering::Relaxed), 0);
        first_task.abort();
        second_task.abort();
    }

    #[tokio::test]
    async fn invalid_pre_meta_chunk_keeps_its_diagnostic_and_stops_failover() {
        let invalid = FakeDownloader {
            invalid_first_chunk: true,
            ..FakeDownloader::default()
        };
        let other = FakeDownloader::default();
        let (first, first_task) = fake_node(invalid.clone()).await;
        let (second, second_task) = fake_node(other.clone()).await;
        first.update_remote_status(0, 2);
        let (router, _) = NodeRouter::with_test_nodes(vec![first, second]);
        let mut deadline = Instant::now() + Duration::from_secs(2);
        let (result, outcome) = track_execution(download_media(&router, None, DownloadRequest::default(), |_| {}, &mut deadline)).await;
        assert!(matches!(result, Err(DownloadErrorKind::InvalidStream)));
        assert!(outcome.is_uncertain());
        assert_eq!(invalid.calls.load(Ordering::Relaxed), 1);
        assert_eq!(other.calls.load(Ordering::Relaxed), 0);
        first_task.abort();
        second_task.abort();
    }

    #[tokio::test]
    async fn stream_requires_completion_after_media_bytes() {
        for missing_completion in [false, true] {
            let service = FakeDownloader {
                missing_completion,
                ..FakeDownloader::default()
            };
            let (node, server) = fake_node(service).await;
            let (router, _) = NodeRouter::with_test_nodes(vec![node]);
            let mut deadline = Instant::now() + Duration::from_secs(2);
            let (result, outcome) = track_execution(async {
                let mut session = download_media(&router, None, DownloadRequest::default(), |_| {}, &mut deadline)
                    .await
                    .unwrap();
                assert!(matches!(session.next_event().await.unwrap(), Some(DownloadEvent::Data(_))));
                session.next_event().await
            })
            .await;
            if missing_completion {
                assert!(matches!(result, Err(DownloadErrorKind::InvalidStream)));
                assert!(outcome.is_uncertain());
            } else {
                assert!(matches!(result, Ok(None)));
                assert!(!outcome.is_uncertain());
            }
            server.abort();
        }
    }

    #[tokio::test]
    async fn completion_marker_is_final_even_if_transport_fails_afterwards() {
        let service = FakeDownloader {
            error_after_completion: true,
            ..FakeDownloader::default()
        };
        let (node, server) = fake_node(service).await;
        let (router, _) = NodeRouter::with_test_nodes(vec![node]);
        let mut deadline = Instant::now() + Duration::from_secs(2);
        let (result, outcome) = track_execution(async {
            let mut session = download_media(&router, None, DownloadRequest::default(), |_| {}, &mut deadline)
                .await
                .unwrap();
            assert!(matches!(session.next_event().await.unwrap(), Some(DownloadEvent::Data(_))));
            assert!(session.next_event().await.unwrap().is_none());
            session.next_event().await
        })
        .await;
        assert!(matches!(result, Ok(None)));
        assert!(!outcome.is_uncertain());
        server.abort();
    }

    #[tokio::test]
    async fn unmarked_internal_stream_error_stops_before_another_node() {
        let failed = FakeDownloader {
            stream_status: Some(Status::internal("Synthetic transport error")),
            ..FakeDownloader::default()
        };
        let other = FakeDownloader::default();
        let (first, first_task) = fake_node(failed.clone()).await;
        let (second, second_task) = fake_node(other.clone()).await;
        first.update_remote_status(0, 2);
        let (router, _) = NodeRouter::with_test_nodes(vec![first, second]);
        let mut deadline = Instant::now() + Duration::from_secs(5);
        let (result, outcome) = track_execution(download_media(&router, None, DownloadRequest::default(), |_| {}, &mut deadline)).await;
        assert!(matches!(result, Err(DownloadErrorKind::ExecutionUncertain)));
        assert!(outcome.is_uncertain());
        assert_eq!(failed.calls.load(Ordering::Relaxed), 1);
        assert_eq!(other.calls.load(Ordering::Relaxed), 0);
        first_task.abort();
        second_task.abort();
    }

    #[tokio::test]
    async fn marked_terminal_stream_error_remains_a_known_candidate_failure() {
        let mut status = Status::internal("Synthetic finished candidate");
        status
            .metadata_mut()
            .insert(proto::TERMINAL_DOWNLOAD_STATUS_HEADER, MetadataValue::from_static("true"));
        let failed = FakeDownloader {
            stream_status: Some(status),
            ..FakeDownloader::default()
        };
        let other = FakeDownloader::default();
        let (first, first_task) = fake_node(failed.clone()).await;
        let (second, second_task) = fake_node(other.clone()).await;
        first.update_remote_status(0, 2);
        let (router, _) = NodeRouter::with_test_nodes(vec![first, second]);
        let mut deadline = Instant::now() + Duration::from_secs(5);
        let result = download_media(&router, None, DownloadRequest::default(), |_| {}, &mut deadline).await;
        assert!(matches!(result, Err(DownloadErrorKind::Rpc(ref status)) if status.code() == Code::Internal));
        assert_eq!(failed.calls.load(Ordering::Relaxed), 1);
        assert_eq!(other.calls.load(Ordering::Relaxed), 0);
        first_task.abort();
        second_task.abort();
    }

    #[tokio::test]
    async fn established_stream_capacity_error_is_not_an_admission_rejection() {
        let (error, outcome) =
            track_execution(async { stream_error(tonic::Status::resource_exhausted("Synthetic stream exhaustion")) }).await;
        assert_eq!(classify_download_error(&error), NodeAttemptErrorKind::ExecutionUncertain);
        assert!(outcome.is_uncertain());
    }

    #[tokio::test]
    async fn transport_codes_are_uncertain_in_both_rpc_phases() {
        for code in [
            Code::Unavailable,
            Code::Unknown,
            Code::Cancelled,
            Code::DeadlineExceeded,
            Code::Internal,
        ] {
            for classify in [start_error, stream_error] {
                let (error, outcome) = track_execution(async { classify(tonic::Status::new(code, "Synthetic transport failure")) }).await;
                assert!(error.is_execution_uncertain(), "{code:?}");
                assert!(outcome.is_uncertain(), "{code:?}");
            }
        }
    }

    #[tokio::test]
    async fn confirmed_terminal_stream_errors_do_not_mark_execution_uncertain() {
        for code in [Code::Aborted, Code::Internal, Code::InvalidArgument, Code::NotFound] {
            let (error, outcome) = track_execution(async {
                let mut status = tonic::Status::new(code, "Synthetic terminal failure");
                status
                    .metadata_mut()
                    .insert(proto::TERMINAL_DOWNLOAD_STATUS_HEADER, MetadataValue::from_static("true"));
                stream_error(status)
            })
            .await;
            assert!(matches!(error, DownloadErrorKind::Rpc(ref status) if status.code() == code));
            assert!(!outcome.is_uncertain(), "{code:?}");
        }
    }

    #[tokio::test]
    async fn admission_rejection_does_not_mark_execution_uncertain() {
        let (error, outcome) =
            track_execution(async { start_error(tonic::Status::resource_exhausted(proto::NODE_CAPACITY_REJECTION_MESSAGE)) }).await;
        assert_eq!(classify_download_error(&error), NodeAttemptErrorKind::ResourceExhausted);
        assert!(!outcome.is_uncertain());
    }

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
        assert!(error.is_execution_uncertain());
        assert_eq!(classify_download_error(&error), NodeAttemptErrorKind::ExecutionUncertain);
    }

    #[test]
    fn only_the_downloader_admission_response_allows_capacity_failover() {
        let error = start_error(tonic::Status::resource_exhausted(proto::NODE_CAPACITY_REJECTION_MESSAGE));
        assert_eq!(classify_download_error(&error), NodeAttemptErrorKind::ResourceExhausted);

        let mut later_error = tonic::Status::resource_exhausted(proto::NODE_CAPACITY_REJECTION_MESSAGE);
        later_error
            .metadata_mut()
            .insert(proto::TERMINAL_DOWNLOAD_STATUS_HEADER, MetadataValue::from_static("true"));
        assert_eq!(classify_download_error(&stream_error(later_error)), NodeAttemptErrorKind::Fatal);
    }

    #[test]
    fn deadline_after_stream_start_is_an_uncertain_execution() {
        let error = stream_error(tonic::Status::deadline_exceeded("synthetic timeout"));
        assert!(error.is_execution_uncertain());
        assert_eq!(classify_download_error(&error), NodeAttemptErrorKind::ExecutionUncertain);
    }
}
