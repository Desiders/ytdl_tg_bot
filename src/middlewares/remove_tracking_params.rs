use froodi::async_impl::Container;
use telers::{
    errors::EventErrorKind,
    event::telegram::HandlerResponse,
    middlewares::{inner::Middleware, Next},
    Request,
};
use tracing::debug;

use crate::{config::TrackingParamsConfig, entities::UrlWithParams};

#[derive(Clone)]
pub struct RemoveTrackingParamsMiddleware;

impl Middleware for RemoveTrackingParamsMiddleware {
    async fn call(&mut self, mut request: Request, next: Next) -> Result<HandlerResponse, EventErrorKind> {
        let container = request.extensions.get::<Container>().unwrap();
        let tracking_params_cfg = container.get::<TrackingParamsConfig>().await.unwrap();

        let Some(UrlWithParams { url, .. }) = request.extensions.get_mut::<UrlWithParams>() else {
            return next(request).await;
        };

        let count_params_before = url.query_pairs().count();
        let params = url
            .query_pairs()
            .filter(|(key, _)| tracking_params_cfg.params.iter().all(|val| &**val != &**key))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect::<Vec<_>>();
        url.query_pairs_mut().clear().extend_pairs(params);
        let count_params_removed = count_params_before - url.query_pairs().count();
        if count_params_removed > 0 {
            debug!(count = count_params_removed, "Tracing params removed");
        }

        next(request).await
    }
}
