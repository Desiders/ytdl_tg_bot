use std::future::Future;
use telers::Request;

pub fn text_empty(request: &mut Request) -> impl Future<Output = bool> {
    let result = if let Some(text) = request.update.text() {
        text.is_empty()
    } else {
        true
    };

    async move { result }
}
