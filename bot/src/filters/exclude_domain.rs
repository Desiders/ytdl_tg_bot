use crate::entities::ChatConfigExcludeDomains;

use std::{convert::Infallible, future::Future};
use telers::{FilterResult, Request};
use url::Url;

pub fn is_exclude_domain(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let chat_cfg = request.extensions.get::<ChatConfigExcludeDomains>().cloned();
    let url = request.extensions.get::<Url>().and_then(|url| url.host().map(|host| host.to_owned()));
    async move {
        let Some(chat_cfg) = chat_cfg else {
            return Ok(false);
        };
        let Some(host) = url else {
            return Ok(false);
        };
        Ok(chat_cfg.0.contains(&host.to_string()))
    }
}
