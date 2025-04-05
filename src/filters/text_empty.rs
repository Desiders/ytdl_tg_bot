use telers::Request;

pub async fn text_empty(request: Request) -> (bool, Request) {
    (
        if let Some(text) = request.update.text() {
            text.is_empty()
        } else {
            true
        },
        request,
    )
}
