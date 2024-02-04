use crate::config;

use std::sync::Arc;
use telers::extractors::FromContext;

#[derive(FromContext)]
#[context(key = "yt_dlp_config", from = Arc<config::YtDlp>)]
pub struct YtDlpWrapper(pub Arc<config::YtDlp>);

impl From<Arc<config::YtDlp>> for YtDlpWrapper {
    fn from(sources: Arc<config::YtDlp>) -> Self {
        Self(sources)
    }
}

#[derive(FromContext)]
#[context(key = "bot_config", from = Arc<config::Bot>)]
pub struct BotConfigWrapper(pub Arc<config::Bot>);

impl From<Arc<config::Bot>> for BotConfigWrapper {
    fn from(sources: Arc<config::Bot>) -> Self {
        Self(sources)
    }
}
