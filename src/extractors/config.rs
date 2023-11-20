use crate::config::{Bot as BotConfig, PhantomVideoId, YtDlp};

use std::sync::Arc;
use telers::{
    client::Bot, context::Context, errors::ExtractionError, extractors::FromEventAndContext, from_context_impl, from_context_into_impl,
    types::Update,
};

pub struct YtDlpWrapper(pub Arc<YtDlp>);

impl From<Arc<YtDlp>> for YtDlpWrapper {
    fn from(sources: Arc<YtDlp>) -> Self {
        Self(sources)
    }
}

pub struct BotConfigWrapper(pub Arc<BotConfig>);

impl From<Arc<BotConfig>> for BotConfigWrapper {
    fn from(sources: Arc<BotConfig>) -> Self {
        Self(sources)
    }
}

from_context_into_impl!([Client], Arc<YtDlp> => YtDlpWrapper, "yt_dlp_config");
from_context_into_impl!([Client], Arc<BotConfig> => BotConfigWrapper, "bot_config");
from_context_impl!([Client], PhantomVideoId, "phantom_video_id");
