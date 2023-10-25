use crate::config::YtDlp;

use std::{ops::Deref, sync::Arc};
use telers::{
    client::Bot, context::Context, errors::ExtractionError, extractors::FromEventAndContext,
    from_context_into_impl, types::Update,
};

pub struct YtDlpWrapper(pub Arc<YtDlp>);

impl Deref for YtDlpWrapper {
    type Target = Arc<YtDlp>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Arc<YtDlp>> for YtDlpWrapper {
    fn from(sources: Arc<YtDlp>) -> Self {
        Self(sources)
    }
}

from_context_into_impl!([Client], Arc<YtDlp> => YtDlpWrapper, "yt_dlp_config");
