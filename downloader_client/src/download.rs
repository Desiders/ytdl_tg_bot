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

pub async fn download_media(
    router: &NodeRouter,
    domain: Option<&str>,
    request: DownloadRequest,
) -> Result<DownloadSession, DownloadErrorKind> {
    with_node_failover(
        router,
        domain,
        |node| {
            let request = request.clone();
            async move {
                let mut client = DownloaderClient::new(node.channel.clone());
                let response = client.download_media(authenticated_request(request, &node.token)?).await?;
                let mut stream = response.into_inner();

                let first = stream
                    .message()
                    .await
                    .map_err(DownloadErrorKind::from)?
                    .ok_or(DownloadErrorKind::InvalidStream)?;
                let Payload::Meta(meta) = first.payload.ok_or(DownloadErrorKind::InvalidStream)? else {
                    return Err(DownloadErrorKind::InvalidStream);
                };

                Ok::<_, DownloadErrorKind>(DownloadSession { meta, stream })
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
