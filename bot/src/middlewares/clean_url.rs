use crate::utils::UrlCleaner;

use froodi::async_impl::Container;
use telers::{
    errors::EventErrorKind,
    event::telegram::HandlerResponse,
    middlewares::{inner::Middleware, Next},
    Request,
};
use url::Url;

#[derive(Clone)]
pub struct CleanUrlMiddleware;

impl Middleware for CleanUrlMiddleware {
    async fn call(&mut self, mut request: Request, next: Next) -> Result<HandlerResponse, EventErrorKind> {
        let container = request.extensions.get::<Container>().unwrap();
        let cleaner = container.get::<UrlCleaner>().await.unwrap();

        let Some(url) = request.extensions.get_mut::<Url>() else {
            return next(request).await;
        };

        if let Some(cleaned) = cleaner.clean(url) {
            *url = cleaned;
        }

        next(request).await
    }
}
