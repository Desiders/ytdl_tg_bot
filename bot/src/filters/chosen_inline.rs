use std::{convert::Infallible, future::Future};
use telers::{FilterResult, Request};

pub fn is_video(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let result = if let Some(result_id) = request.update.result_id() {
        result_id.starts_with("video_")
    } else {
        false
    };
    async move { Ok(result) }
}

pub fn is_audio(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let result = if let Some(result_id) = request.update.result_id() {
        result_id.starts_with("audio_")
    } else {
        false
    };
    async move { Ok(result) }
}
