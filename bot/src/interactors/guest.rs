use std::sync::Arc;

use rust_i18n::t;
use telers::errors::HandlerError;
use tracing::{info, warn};
use url::Url;

use crate::{
    config::Config,
    entities::{ChatConfig, DownloadJob, JobTarget, Params},
    interactors::Interactor,
    locale::Locale,
    services::{
        messenger::{AnswerGuestRequest, EditTarget, EditTextRequest, MessengerPort},
        queue::{GuestQueuePort, RedisJobQueue},
    },
    utils::UrlCleaner,
    value_objects::MediaType,
};

pub struct EnqueueGuestDownload<Messenger, Queue = RedisJobQueue> {
    messenger: Arc<Messenger>,
    queue: Arc<Queue>,
    cleaner: Arc<UrlCleaner>,
    cfg: Arc<Config>,
}

impl<Messenger, Queue> EnqueueGuestDownload<Messenger, Queue> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>, queue: Arc<Queue>, cleaner: Arc<UrlCleaner>, cfg: Arc<Config>) -> Self {
        Self {
            messenger,
            queue,
            cleaner,
            cfg,
        }
    }

    fn prepare_url(&self, url: Url) -> Option<Url> {
        let url = self.cleaner.clean(&url).unwrap_or(url);
        (matches!(url.scheme(), "http" | "https")
            && url.domain().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && !self
                .cfg
                .blacklisted
                .domains
                .iter()
                .any(|domain| url.domain() == Some(domain.as_str())))
        .then_some(url)
    }
}

pub struct GuestInput<'a> {
    pub query_id: &'a str,
    pub url: Option<Url>,
    pub locale: Locale,
    /// No explicit command means automatic media classification.
    pub media_type: Option<MediaType>,
}

impl<Messenger: MessengerPort, Queue: GuestQueuePort> Interactor<GuestInput<'_>> for &EnqueueGuestDownload<Messenger, Queue> {
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: GuestInput<'_>) -> Result<(), HandlerError> {
        // Fail closed if Redis cannot establish ownership. Do not send an untracked reply.
        match self.queue.reserve_guest_query(input.query_id).await {
            Ok(true) => {}
            Ok(false) => return Ok(()),
            Err(_) => {
                warn!("Could not reserve guest query");
                return Ok(());
            }
        }

        let url = input.url.and_then(|url| self.prepare_url(url));
        let text = if url.is_some() {
            t!("download.preparing", locale = input.locale.as_str())
        } else {
            t!("guest.help", locale = input.locale.as_str())
        };
        let reply = self
            .messenger
            .answer_guest(AnswerGuestRequest {
                query_id: input.query_id,
                text: &text,
            })
            .await;
        let inline_message_id = match reply {
            Ok(id) => id,
            Err(err) => {
                // Keep the reservation: the reply may have been delivered.
                warn!(category = err.category(), "Guest reply failed or its outcome is unknown");
                return Ok(());
            }
        };
        let Some(url) = url else {
            return Ok(());
        };
        let job = guest_job(url, &inline_message_id, input.locale, input.media_type);
        if self.queue.enqueue_guest(&job).await.is_err() {
            // XADD can also have an ambiguous outcome: no local retry or replacement job.
            let text = t!("guest.queue_unknown", locale = input.locale.as_str());
            let _ = self
                .messenger
                .edit_text(EditTextRequest {
                    is_progress: false,
                    target: EditTarget::InlineMessage {
                        inline_message_id: &inline_message_id,
                    },
                    text: &text,
                    format: None,
                    disable_link_preview: true,
                    clear_inline_keyboard: false,
                })
                .await;
            warn!("Could not confirm guest download enqueue");
        } else {
            info!(job_id = %job.job_id, "Enqueued guest download");
        }
        Ok(())
    }
}

fn guest_job(url: Url, inline_message_id: &str, locale: Locale, media_type: Option<MediaType>) -> DownloadJob {
    let mut job = DownloadJob::new(
        media_type.unwrap_or(MediaType::Video),
        Some(url),
        Params::default(),
        // Guest chats have no database record; download inputs still require a ChatConfig.
        ChatConfig::new(0, false, locale.as_str().into()),
        false,
        JobTarget::Inline {
            inline_message_id: inline_message_id.to_owned(),
            result_id: "guest".into(),
        },
    );
    job.auto = media_type.is_none();
    job.guest = true;
    job
}
