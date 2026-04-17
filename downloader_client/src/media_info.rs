use proto::downloader::{downloader_client::DownloaderClient, MediaInfoRequest, MediaInfoResponse};
use tonic::Code;

use crate::{authenticated_request, with_node_failover, GetMediaInfoErrorKind, NodeAttemptErrorKind, NodeRouter};

const MAX_DECODING_MESSAGE_SIZE: usize = 30 * 1024 * 1024;

pub async fn get_media_info(
    router: &NodeRouter,
    domain: Option<&str>,
    request: MediaInfoRequest,
) -> Result<MediaInfoResponse, GetMediaInfoErrorKind> {
    with_node_failover(
        router,
        domain,
        |node| {
            let request = request.clone();
            async move {
                let mut client = DownloaderClient::new(node.channel.clone()).max_decoding_message_size(MAX_DECODING_MESSAGE_SIZE);
                let response = client.get_media_info(authenticated_request(request, &node.token)?).await?;
                Ok::<_, GetMediaInfoErrorKind>(response.into_inner())
            }
        },
        classify_get_media_info_error,
    )
    .await
    .map_err(GetMediaInfoErrorKind::from)
}

fn classify_get_media_info_error(err: &GetMediaInfoErrorKind) -> NodeAttemptErrorKind {
    match err {
        GetMediaInfoErrorKind::Rpc(status) if status.code() == Code::ResourceExhausted => NodeAttemptErrorKind::ResourceExhausted,
        GetMediaInfoErrorKind::Rpc(status) if status.code() == Code::Aborted => NodeAttemptErrorKind::ContextUnavailable,
        GetMediaInfoErrorKind::Rpc(status) if status.code() == Code::Unavailable => NodeAttemptErrorKind::Unavailable,
        GetMediaInfoErrorKind::Rpc(status) if status.code() == Code::Unauthenticated => NodeAttemptErrorKind::Unauthenticated,
        _ => NodeAttemptErrorKind::Fatal,
    }
}
