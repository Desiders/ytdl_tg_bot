use std::future::Future;
use telers::Request;

#[allow(clippy::module_name_repetitions)]
pub fn is_via_bot(request: &mut Request) -> impl Future<Output = bool> {
    let result = request.update.message().and_then(|message| message.via_bot()).is_some();
    async move { result }
}
