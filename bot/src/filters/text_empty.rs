use std::{convert::Infallible, future::Future};
use telers::{FilterResult, Request};

pub fn text_empty(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let result = if let Some(text) = request.update.text().or(request.update.query()) {
        text.is_empty()
    } else {
        true
    };

    async move { Ok(result) }
}
