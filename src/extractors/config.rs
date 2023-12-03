use crate::config::{Bot as BotConfig, PhantomAudioId, PhantomVideoId, YtDlp};

use std::sync::Arc;
use telers::{
    client::Bot, context::Context, errors::ExtractionError, extractors::FromEventAndContext, from_context, from_context_into, types::Update,
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

from_context_into!([Client], Arc<YtDlp> => YtDlpWrapper, "yt_dlp_config");
from_context_into!([Client], Arc<BotConfig> => BotConfigWrapper, "bot_config");
from_context!([Client], PhantomVideoId, "phantom_video_id");
from_context!([Client], PhantomAudioId, "phantom_audio_id");
