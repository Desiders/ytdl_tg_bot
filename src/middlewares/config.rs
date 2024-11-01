use crate::config::{Bot as BotConfig, YtDlp};

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
    bot: Arc<BotConfig>,
}

impl Config {
    pub fn new(yt_dlp: YtDlp, bot: BotConfig) -> Self {
        Self {
            yt_dlp: Arc::new(yt_dlp),
            bot: Arc::new(bot),
        }
    }
}

#[async_trait]
impl<Client> OuterMiddleware<Client> for Config
where
    Client: Send + Sync + 'static,
{
    async fn call(&self, request: Request<Client>) -> Result<MiddlewareResponse<Client>, EventErrorKind> {
        request.context.insert("yt_dlp_config", Box::new(self.yt_dlp.clone()));
        request.context.insert("bot_config", Box::new(self.bot.clone()));

        Ok((request, EventReturn::Finish))
    }
}
