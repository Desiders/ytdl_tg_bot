use std::future::Future;
use telers::{
    types::{ChosenInlineResult, UpdateKind},
    Request,
};

pub fn is_video(request: &mut Request) -> impl Future<Output = bool> {
    let result = if let UpdateKind::ChosenInlineResult(ChosenInlineResult { result_id, .. }) = request.update.kind() {
        result_id.starts_with("video_")
    } else {
        false
    };
    async move { result }
}

pub fn is_audio(request: &mut Request) -> impl Future<Output = bool> {
    let result = if let UpdateKind::ChosenInlineResult(ChosenInlineResult { result_id, .. }) = request.update.kind() {
        result_id.starts_with("audio_")
    } else {
        false
    };
    async move { result }
}
