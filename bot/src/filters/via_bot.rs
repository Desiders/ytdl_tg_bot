use std::{convert::Infallible, future::Future};
use telers::{FilterResult, Request};

#[allow(clippy::module_name_repetitions)]
pub fn is_via_bot(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let result = request.update.message().and_then(|message| message.via_bot()).is_some();
    async move { Ok(result) }
}
