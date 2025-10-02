use async_trait::async_trait;
use froodi::{async_impl::Container, Context, DefaultScope};
use telers::{
    errors::EventErrorKind,
    event::telegram::HandlerResponse,
    middlewares::{inner::Middleware, Next},
    Request,
};

#[derive(Clone)]
pub struct ContainerMiddleware {
    pub container: Container,
}

#[async_trait]
impl Middleware for ContainerMiddleware {
    async fn call(&mut self, mut request: Request, next: Next) -> Result<HandlerResponse, EventErrorKind> {
        let mut context = Context::new();
        context.insert(request.update.clone());

        let container = self
            .container
            .clone()
            .enter()
            .with_scope(DefaultScope::Request)
            .with_context(context)
            .build()
            .unwrap();
        request.extensions.insert(container.clone());

        let resp = next(request).await;

        container.close().await;

        resp
    }
}
