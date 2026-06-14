use std::sync::Arc;

use proto::downloader::{song_recognizer_server::SongRecognizer, RecognizeSongRequest, RecognizeSongResponse};
use tonic::{Request, Response, Status};
use tracing::error;

use crate::services::songrec::{RecognizeErrorKind, SongRecognizer as Recognizer};

pub struct SongRecognizerService {
    pub recognizer: Arc<Recognizer>,
}

#[tonic::async_trait]
impl SongRecognizer for SongRecognizerService {
    async fn recognize_song(&self, request: Request<RecognizeSongRequest>) -> Result<Response<RecognizeSongResponse>, Status> {
        let audio = request.into_inner().audio;

        let recognized = self.recognizer.recognize(&audio).await.map_err(|err| {
            error!(%err, "Recognize song failed");
            recognize_error_status(err)
        })?;

        Ok(Response::new(RecognizeSongResponse {
            title: recognized.title,
            artist: recognized.artist,
            album: recognized.album,
            url: recognized.url,
            cover_url: recognized.cover_url,
        }))
    }
}

fn recognize_error_status(err: RecognizeErrorKind) -> Status {
    match err {
        RecognizeErrorKind::EmptyAudio | RecognizeErrorKind::Decode => Status::invalid_argument(err.to_string()),
        RecognizeErrorKind::NoMatch => Status::not_found(err.to_string()),
        RecognizeErrorKind::Disabled => Status::failed_precondition(err.to_string()),
        RecognizeErrorKind::Io(_) | RecognizeErrorKind::Parse(_) | RecognizeErrorKind::Timeout => Status::internal(err.to_string()),
    }
}
