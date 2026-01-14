use crate::entities::ChatConfig;

use std::future::Future;
use telers::Request;

pub fn random_cmd_is_enabled(request: &mut Request) -> impl Future<Output = bool> {
    let chat_cfg = request.extensions.get::<ChatConfig>().cloned();
    async move {
        return chat_cfg.map(|cfg| cfg.cmd_random_enabled).unwrap_or(false);
    }
}
