//! Fast-path interactors that put a download on the Redis queue instead of running it inline.
//!
//! They emit the immediate "⏳ queued" feedback and persist a [`DownloadJob`]; a worker pool drains
//! the queue and runs the real download interactor ([`crate::interactors::video::Download`] etc.).

use std::sync::Arc;

use rust_i18n::t;
use telers::errors::HandlerError;
use tracing::{error, instrument};
use url::Url;

use crate::{
    entities::{ChatConfig, DownloadJob, JobTarget, Params},
    handlers_utils::progress,
    interactors::Interactor,
    services::{
        messenger::{EditTarget, EditTextRequest, MessengerPort, TextFormat},
        queue::RedisJobQueue,
    },
    utils::ErrorFormatter,
    value_objects::MediaType,
};

pub struct EnqueueCommandDownload<Messenger> {
    error_formatter: Arc<ErrorFormatter>,
    messenger: Arc<Messenger>,
    queue: Arc<RedisJobQueue>,
}

impl<Messenger> EnqueueCommandDownload<Messenger> {
    #[must_use]
    pub const fn new(error_formatter: Arc<ErrorFormatter>, messenger: Arc<Messenger>, queue: Arc<RedisJobQueue>) -> Self {
        Self {
            error_formatter,
            messenger,
            queue,
        }
    }
}

pub struct EnqueueCommandInput<'a> {
    pub media_type: MediaType,
    pub chat_id: i64,
    pub message_id: i64,
    pub url: &'a Url,
    pub params: &'a Params,
    pub chat_cfg: &'a ChatConfig,
    pub link_is_visible: bool,
}

impl<Messenger> Interactor<EnqueueCommandInput<'_>> for &EnqueueCommandDownload<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(chat_id = input.chat_id, message_id = input.message_id, url = input.url.as_str()))]
    async fn execute(self, input: EnqueueCommandInput<'_>) -> Result<Self::Output, Self::Err> {
        let locale = input.chat_cfg.locale();
        let queued_text = t!("download.queued", locale = locale.as_str()).into_owned();

        let queued = match progress::new(self.messenger.as_ref(), &queued_text, input.chat_id, Some(input.message_id), None).await {
            Ok(message) => message,
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Send queued message error");
                return Ok(());
            }
        };

        let job = DownloadJob::new(
            input.media_type,
            Some(input.url.clone()),
            input.params.clone(),
            input.chat_cfg.clone(),
            input.link_is_visible,
            JobTarget::Command {
                chat_id: input.chat_id,
                message_id: input.message_id,
                queued_message_id: queued.message_id,
            },
        );

        if let Err(err) = self.queue.enqueue(&job).await {
            error!(%err, "Enqueue download job error");
            let text = t!("download.error_queue", locale = locale.as_str()).into_owned();
            let _ = progress::is_error_in_progress(
                self.messenger.as_ref(),
                input.chat_id,
                queued.message_id,
                &text,
                Some(TextFormat::Html),
            )
            .await;
        }

        Ok(())
    }
}

pub struct EnqueueInlineDownload<Messenger> {
    messenger: Arc<Messenger>,
    queue: Arc<RedisJobQueue>,
}

impl<Messenger> EnqueueInlineDownload<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>, queue: Arc<RedisJobQueue>) -> Self {
        Self { messenger, queue }
    }
}

pub struct EnqueueInlineInput<'a> {
    pub media_type: MediaType,
    pub inline_message_id: &'a str,
    pub result_id: &'a str,
    pub url: Option<&'a Url>,
    pub params: &'a Params,
    pub chat_cfg: &'a ChatConfig,
    pub link_is_visible: bool,
}

impl<Messenger> Interactor<EnqueueInlineInput<'_>> for &EnqueueInlineDownload<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(inline_message_id = input.inline_message_id, result_id = input.result_id))]
    async fn execute(self, input: EnqueueInlineInput<'_>) -> Result<Self::Output, Self::Err> {
        let locale = input.chat_cfg.locale();
        let queued_text = t!("download.queued", locale = locale.as_str()).into_owned();

        let _ = self
            .messenger
            .edit_text(EditTextRequest {
                target: EditTarget::InlineMessage {
                    inline_message_id: input.inline_message_id,
                },
                text: &queued_text,
                format: None,
                disable_link_preview: true,
                clear_inline_keyboard: false,
            })
            .await;

        let job = DownloadJob::new(
            input.media_type,
            input.url.cloned(),
            input.params.clone(),
            input.chat_cfg.clone(),
            input.link_is_visible,
            JobTarget::Inline {
                inline_message_id: input.inline_message_id.to_owned(),
                result_id: input.result_id.to_owned(),
            },
        );

        if let Err(err) = self.queue.enqueue(&job).await {
            error!(%err, "Enqueue inline download job error");
            let text = t!("download.error_queue", locale = locale.as_str()).into_owned();
            let _ = self
                .messenger
                .edit_text(EditTextRequest {
                    target: EditTarget::InlineMessage {
                        inline_message_id: input.inline_message_id,
                    },
                    text: &text,
                    format: Some(TextFormat::Html),
                    disable_link_preview: true,
                    clear_inline_keyboard: false,
                })
                .await;
        }

        Ok(())
    }
}
