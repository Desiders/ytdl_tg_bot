//! A durable unit of download work pulled off the Redis queue by a worker.
//!
//! Carries everything a worker needs to rebuild the original interactor input without the source
//! `Update`: where to send the result ([`JobTarget`]), the URL, the parsed [`Params`] and the
//! chat's [`ChatConfig`].

use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::{
    entities::{ChatConfig, Params},
    value_objects::MediaType,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadJob {
    pub job_id: Uuid,
    pub media_type: MediaType,
    /// Always `Some` for [`JobTarget::Command`]; may be `None` for inline (URL came via the result).
    pub url: Option<Url>,
    pub params: Params,
    pub chat_cfg: ChatConfig,
    pub link_is_visible: bool,
    pub target: JobTarget,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub progress_message_id: Option<i64>,
    #[serde(default)]
    pub base_text: Option<String>,
}

impl DownloadJob {
    /// Builds a fresh job — new `job_id`, zero attempts — for the given target.
    #[must_use]
    pub fn new(
        media_type: MediaType,
        url: Option<Url>,
        params: Params,
        chat_cfg: ChatConfig,
        link_is_visible: bool,
        target: JobTarget,
    ) -> Self {
        Self {
            job_id: Uuid::now_v7(),
            media_type,
            url,
            params,
            chat_cfg,
            link_is_visible,
            target,
            attempts: 0,
            progress_message_id: None,
            base_text: None,
        }
    }

    #[must_use]
    pub fn with_progress_reuse(mut self, progress_message_id: i64, base_text: Option<String>) -> Self {
        self.progress_message_id = Some(progress_message_id);
        self.base_text = base_text;
        self
    }
}

/// Where the worker delivers the result and which interactor it routes to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobTarget {
    Command { chat_id: i64, message_id: i64 },
    Inline { inline_message_id: String, result_id: String },
}
