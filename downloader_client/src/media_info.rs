use std::collections::HashSet;

use proto::downloader::{downloader_client::DownloaderClient, MediaInfoRequest, MediaInfoResponse};
use tonic::Code;
use tracing::info;

use crate::{
    authenticated_request, outcome::record_execution_progress, with_node_failover, GetMediaInfoErrorKind, NodeAttemptErrorKind,
    NodeFailoverError, NodeRouter,
};

const MAX_DECODING_MESSAGE_SIZE: usize = 30 * 1024 * 1024;

/// Fetches media information from a routed downloader node.
///
/// # Errors
///
/// Returns an error if no node is available, authentication metadata cannot be
/// built, or the RPC fails.
pub async fn get_media_info(
    router: &NodeRouter,
    domain: Option<&str>,
    request: MediaInfoRequest,
) -> Result<MediaInfoResponse, GetMediaInfoErrorKind> {
    let mut capacity = router.subscribe_capacity();
    let mut waiting_for_node = false;
    loop {
        capacity.borrow_and_update();
        let result = with_node_failover(
            router,
            domain,
            |node| {
                let request = request.clone();
                async move {
                    let mut client = DownloaderClient::new(node.channel.clone()).max_decoding_message_size(MAX_DECODING_MESSAGE_SIZE);
                    let response = client.get_media_info(authenticated_request(request, &node.token)?).await?;
                    record_execution_progress();
                    Ok::<_, GetMediaInfoErrorKind>(response.into_inner())
                }
            },
            classify_get_media_info_error,
        )
        .await;
        match result {
            Err(NodeFailoverError::AllNodesBusy) => {}
            Err(NodeFailoverError::NodeUnavailable) if router.pick_node(domain, &HashSet::new()).is_none() => {}
            other => return other.map_err(GetMediaInfoErrorKind::from),
        }
        if !waiting_for_node {
            info!("No node can fetch media information; waiting for a status change");
            waiting_for_node = true;
        }
        if !NodeRouter::wait_for_status_change(&mut capacity).await {
            return Err(GetMediaInfoErrorKind::NodeUnavailable);
        }
    }
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
