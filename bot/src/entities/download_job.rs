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
#[allow(
    clippy::struct_excessive_bools,
    reason = "Keep independent flags compatible with existing queued JSON payloads"
)]
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
    pub progress_message_id: Option<i64>,
    #[serde(default)]
    pub base_text: Option<String>,
    /// Auto jobs carry no committed media type: the worker classifies (video -> audio -> photo) and
    /// runs the auto download. `media_type` is an ignored placeholder for these.
    #[serde(default)]
    pub auto: bool,
    /// For auto jobs: run silently (no progress message). Set for group chats.
    #[serde(default)]
    pub quiet: bool,
    /// Guest replies use inline delivery with default settings and no diagnostic details.
    #[serde(default)]
    pub guest: bool,
}

impl DownloadJob {
    /// Builds a fresh job with a new `job_id` for the given target.
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
            progress_message_id: None,
            base_text: None,
            auto: false,
            quiet: false,
            guest: false,
        }
    }

    #[must_use]
    pub fn with_auto(mut self, quiet: bool) -> Self {
        self.auto = true;
        self.quiet = quiet;
        self
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_attempt_metadata_does_not_prevent_delivery() {
        let job = DownloadJob::new(
            MediaType::Video,
            Some(Url::parse("https://example.com/media").unwrap()),
            Params::default(),
            ChatConfig::new(1, false, "en".into()),
            true,
            JobTarget::Command { chat_id: 1, message_id: 2 },
        );
        let mut payload = serde_json::to_value(&job).unwrap();
        payload["attempts"] = 2.into();
        payload.as_object_mut().unwrap().remove("guest");
        let decoded: DownloadJob = serde_json::from_value(payload).unwrap();
        assert!(!decoded.guest);
        assert_eq!(decoded.job_id, job.job_id);
        assert_eq!(decoded.url, job.url);
        assert!(serde_json::to_value(decoded).unwrap().get("attempts").is_none());
    }
}
