use std::sync::Arc;

use rust_i18n::t;
use telers::errors::HandlerError;
use tracing::{error, info, instrument};
use url::Url;

use crate::{
    entities::{ChatConfig, DownloadJob, JobTarget, Params},
    handlers_utils::progress,
    interactors::Interactor,
    services::{
        messenger::{EditTarget, EditTextRequest, MessengerPort, TextFormat},
        queue::RedisJobQueue,
    },
    value_objects::MediaType,
};

pub struct EnqueueCommandDownload<Messenger> {
    messenger: Arc<Messenger>,
    queue: Arc<RedisJobQueue>,
}

impl<Messenger> EnqueueCommandDownload<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>, queue: Arc<RedisJobQueue>) -> Self {
        Self { messenger, queue }
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
    pub progress_message_id: Option<i64>,
    pub base_text: Option<&'a str>,
    /// When set, the worker classifies the link and runs the auto download (`media_type` is then an
    /// ignored placeholder).
    pub auto: bool,
    /// For auto jobs: run silently (group chats). Ignored when `auto` is false.
    pub quiet: bool,
}

impl<Messenger> Interactor<EnqueueCommandInput<'_>> for &EnqueueCommandDownload<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(chat_id = input.chat_id, message_id = input.message_id, url = input.url.as_str()))]
    async fn execute(self, input: EnqueueCommandInput<'_>) -> Result<Self::Output, Self::Err> {
        let mut job = DownloadJob::new(
            input.media_type,
            Some(input.url.clone()),
            input.params.clone(),
            input.chat_cfg.clone(),
            input.link_is_visible,
            JobTarget::Command {
                chat_id: input.chat_id,
                message_id: input.message_id,
            },
        );
        if input.auto {
            job = job.as_auto(input.quiet);
        }
        if let Some(progress_message_id) = input.progress_message_id {
            job = job.with_progress_reuse(progress_message_id, input.base_text.map(ToOwned::to_owned));
        }

        info!(job_id = %job.job_id, "Enqueueing download job");
        if let Err(err) = self.queue.enqueue(&job).await {
            error!(%err, "Enqueue download job error");
            let text = t!("download.error_queue", locale = input.chat_cfg.locale().as_str()).into_owned();
            let _ = progress::new(self.messenger.as_ref(), &text, input.chat_id, Some(input.message_id), None).await;
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

        info!(job_id = %job.job_id, "Enqueueing download job");
        if let Err(err) = self.queue.enqueue(&job).await {
            error!(%err, "Enqueue inline download job error");
            let text = t!("download.error_queue", locale = input.chat_cfg.locale().as_str()).into_owned();
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
