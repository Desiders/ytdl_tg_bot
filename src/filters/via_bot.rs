use std::future::Future;
use telers::Request;

#[allow(clippy::module_name_repetitions)]
pub fn is_via_bot(request: &mut Request) -> impl Future<Output = bool> {
    let result = request.update.message().map(|message| message.via_bot()).flatten().is_some();

    async move { result }
}
