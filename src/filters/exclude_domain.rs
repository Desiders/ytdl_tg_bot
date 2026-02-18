use crate::entities::ChatConfigExcludeDomains;

use std::future::Future;
use telers::Request;
use url::Url;

pub fn is_exclude_domain(request: &mut Request) -> impl Future<Output = bool> {
    let chat_cfg = request.extensions.get::<ChatConfigExcludeDomains>().cloned();
    let url = request
        .extensions
        .get::<Url>()
        .map(|url| url.host().map(|host| host.to_owned()))
        .flatten();
    async move {
        let Some(chat_cfg) = chat_cfg else {
            return false;
        };
        let Some(host) = url else {
            return false;
        };
        chat_cfg.0.contains(&host.to_string())
    }
}
