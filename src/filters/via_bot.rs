use telers::Request;

#[allow(clippy::module_name_repetitions)]
pub async fn is_via_bot(request: Request) -> (bool, Request) {
    (request.update.message().and_then(|message| message.via_bot()).is_some(), request)
}
