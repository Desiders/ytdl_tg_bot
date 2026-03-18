use crate::entities::ChatConfig;

use std::{convert::Infallible, future::Future};
use telers::{FilterResult, Request};

pub fn random_cmd_is_enabled(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let chat_cfg = request.extensions.get::<ChatConfig>().cloned();
    async move { Ok(chat_cfg.is_some_and(|cfg| cfg.cmd_random_enabled)) }
}
