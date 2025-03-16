use crate::config::{Bot as BotConfig, YtDlp};

use async_trait::async_trait;
use telers::{
    errors::EventErrorKind,
    event::EventReturn,
    middlewares::{outer::MiddlewareResponse, OuterMiddleware},
    Request,
};

#[derive(Clone, Debug)]
pub struct Config {
    yt_dlp: YtDlp,
    bot: BotConfig,
}

impl Config {
    pub fn new(yt_dlp: YtDlp, bot: BotConfig) -> Self {
        Self { yt_dlp, bot }
    }
}

#[async_trait]
impl<Client> OuterMiddleware<Client> for Config
where
    Client: Send + Sync + 'static,
{
    async fn call(&mut self, mut request: Request<Client>) -> Result<MiddlewareResponse<Client>, EventErrorKind> {
        request.extensions.insert(self.yt_dlp.clone());
        request.extensions.insert(self.bot.clone());

        Ok((request, EventReturn::Finish))
    }
}
