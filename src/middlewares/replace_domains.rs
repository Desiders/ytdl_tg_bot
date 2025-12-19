use froodi::async_impl::Container;
use regex::Regex;
use telers::{
    errors::EventErrorKind,
    event::telegram::HandlerResponse,
    middlewares::{inner::Middleware, Next},
    Request,
};
use tracing::info;

use crate::{config::ReplaceDomainsConfig, entities::UrlWithParams};

#[derive(Clone)]
pub struct ReplaceDomainsMiddleware;

impl Middleware for ReplaceDomainsMiddleware {
    async fn call(&mut self, mut request: Request, next: Next) -> Result<HandlerResponse, EventErrorKind> {
        let container = request.extensions.get::<Container>().unwrap();
        let replace_domains = container.get::<Vec<ReplaceDomainsConfig>>().await.unwrap();

        let Some(UrlWithParams { url, .. }) = request.extensions.get_mut::<UrlWithParams>() else {
            return next(request).await;
        };
        let Some(domain) = url.domain().map(ToOwned::to_owned) else {
            return next(request).await;
        };

        for ReplaceDomainsConfig { from, to } in replace_domains.iter() {
            let re = Regex::new(from).expect("invalid `from` pattern");
            if re.is_match(&domain) {
                info!(from = domain, to, "Replace domain");
                url.set_host(Some(&re.replace(&domain, &**to))).expect("invalid host");
                break;
            }
        }

        next(request).await
    }
}
