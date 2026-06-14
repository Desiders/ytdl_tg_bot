use proto::downloader::{song_recognizer_client::SongRecognizerClient, RecognizeSongRequest};
use tonic::Code;
use tracing::{info, instrument};

use crate::{authenticated_request, with_node_failover, NodeAttemptErrorKind, NodeRouter, RecognizeSongErrorKind};

const MAX_ENCODING_MESSAGE_SIZE: usize = 25 * 1024 * 1024;

/// A song recognized from an audio clip (Shazam, via SongRec on a node).
#[derive(Debug, Clone)]
pub struct RecognizedSong {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub url: Option<String>,
    pub cover_url: Option<String>,
}

/// Recognizes a song from a short audio clip via a routed downloader node.
///
/// # Errors
///
/// Returns an error if no node is available, authentication metadata cannot be built, the RPC
/// fails, or no song matched (`RecognizeSongErrorKind::is_no_match`).
#[instrument(skip_all, fields(bytes = audio.len()))]
pub async fn recognize_song(router: &NodeRouter, audio: Vec<u8>) -> Result<RecognizedSong, RecognizeSongErrorKind> {
    let response = with_node_failover(
        router,
        None,
        |node| {
            let audio = audio.clone();
            async move {
                let mut client = SongRecognizerClient::new(node.channel.clone()).max_encoding_message_size(MAX_ENCODING_MESSAGE_SIZE);
                let response = client
                    .recognize_song(authenticated_request(RecognizeSongRequest { audio }, &node.token)?)
                    .await?;
                Ok::<_, RecognizeSongErrorKind>(response.into_inner())
            }
        },
        classify_recognize_song_error,
    )
    .await
    .map_err(RecognizeSongErrorKind::from)?;

    info!(matched_title = response.title.as_deref(), "Recognized song");
    Ok(RecognizedSong {
        title: response.title,
        artist: response.artist,
        album: response.album,
        url: response.url,
        cover_url: response.cover_url,
    })
}

fn classify_recognize_song_error(err: &RecognizeSongErrorKind) -> NodeAttemptErrorKind {
    match err {
        RecognizeSongErrorKind::Rpc(status) if status.code() == Code::ResourceExhausted => NodeAttemptErrorKind::ResourceExhausted,
        RecognizeSongErrorKind::Rpc(status) if status.code() == Code::Unavailable => NodeAttemptErrorKind::Unavailable,
        RecognizeSongErrorKind::Rpc(status) if status.code() == Code::Unauthenticated => NodeAttemptErrorKind::Unauthenticated,
        _ => NodeAttemptErrorKind::Fatal,
    }
}
