use bytes::Bytes;
use proto::downloader::{download_chunk::Payload, downloader_client::DownloaderClient, DownloadChunk, DownloadMeta, DownloadRequest};
use tonic::Code;

use crate::{authenticated_request, with_node_failover, DownloadErrorKind, NodeAttemptErrorKind, NodeRouter};

pub enum DownloadEvent {
    Progress(String),
    Data(Bytes),
    ThumbnailData(Bytes),
}

pub struct DownloadSession {
    meta: DownloadMeta,
    stream: tonic::Streaming<DownloadChunk>,
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
        let Some(chunk) = self.stream.message().await.map_err(DownloadErrorKind::from)? else {
            return Ok(None);
        };

        match chunk.payload {
            Some(Payload::Progress(progress)) => Ok(Some(DownloadEvent::Progress(progress))),
            Some(Payload::Data(data)) => Ok(Some(DownloadEvent::Data(Bytes::from(data)))),
            Some(Payload::ThumbnailData(data)) => Ok(Some(DownloadEvent::ThumbnailData(Bytes::from(data)))),
            Some(Payload::Meta(_)) | None => Err(DownloadErrorKind::InvalidStream),
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
    with_node_failover(
        router,
        domain,
        |node| {
            let request = request.clone();
            async move {
                let mut client = DownloaderClient::new(node.channel.clone());
                let response = client.download_media(authenticated_request(request, &node.token)?).await?;
                let mut stream = response.into_inner();

                loop {
                    let chunk = stream
                        .message()
                        .await
                        .map_err(DownloadErrorKind::from)?
                        .ok_or(DownloadErrorKind::InvalidStream)?;
                    match chunk.payload.ok_or(DownloadErrorKind::InvalidStream)? {
                        Payload::Progress(progress) => on_progress(progress),
                        Payload::Meta(meta) => return Ok::<_, DownloadErrorKind>(DownloadSession { meta, stream }),
                        Payload::Data(_) | Payload::ThumbnailData(_) => return Err(DownloadErrorKind::InvalidStream),
                    }
                }
            }
        },
        classify_download_error,
    )
    .await
    .map_err(DownloadErrorKind::from)
}

fn classify_download_error(err: &DownloadErrorKind) -> NodeAttemptErrorKind {
    match err {
        DownloadErrorKind::Rpc(status) if status.code() == Code::ResourceExhausted => NodeAttemptErrorKind::ResourceExhausted,
        DownloadErrorKind::Rpc(status) if status.code() == Code::Aborted => NodeAttemptErrorKind::ContextUnavailable,
        DownloadErrorKind::Rpc(status) if status.code() == Code::Unavailable => NodeAttemptErrorKind::Unavailable,
        DownloadErrorKind::Rpc(status) if status.code() == Code::Unauthenticated => NodeAttemptErrorKind::Unauthenticated,
        _ => NodeAttemptErrorKind::Fatal,
    }
}
