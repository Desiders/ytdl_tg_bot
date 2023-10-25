use crate::config::YtDlp;

use async_trait::async_trait;
use std::sync::Arc;
use telers::{
    errors::EventErrorKind,
    event::EventReturn,
    middlewares::{outer::MiddlewareResponse, OuterMiddleware},
    router::Request,
};

#[derive(Clone, Debug)]
pub struct Config {
    yt_dlp: Arc<YtDlp>,
}

impl Config {
    pub fn new(yt_dlp: YtDlp) -> Self {
        Self {
            yt_dlp: Arc::new(yt_dlp),
        }
    }
}

#[async_trait]
impl<Client> OuterMiddleware<Client> for Config
where
    Client: Send + Sync + 'static,
{
    async fn call(
        &self,
        request: Request<Client>,
    ) -> Result<MiddlewareResponse<Client>, EventErrorKind> {
        request
            .context
            .insert("yt_dlp_config", Box::new(self.yt_dlp.clone()));

        Ok((request, EventReturn::Finish))
    }
}
