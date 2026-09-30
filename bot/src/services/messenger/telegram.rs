use crate::{
    config::TimeoutsConfig,
    entities::{MediaByteStream, MediaForUpload},
    services::{messenger::MessengerPort, progress_throttle::ProgressThrottle},
    utils::{media_link, sanitize_send_filename, ErrorFormatter},
};

use backoff::ExponentialBackoff;
use bytes::Bytes;
use downloader_client::outcome::{current_execution_outcome, record_execution_progress};
use futures_util::{Stream, StreamExt as _};
use std::{
    future::Future,
    io,
    sync::{
        atomic::{AtomicU8, Ordering::Relaxed},
        Arc,
    },
    time::Duration,
};
use telers::{
    enums::ParseMode,
    errors::{SessionErrorKind, TelegramErrorKind},
    methods::{self, AnswerInlineQuery, DeleteMessage, EditMessageText, GetMe, SendMediaGroup, SendMessage, TelegramMethod},
    types::{
        ChatIdKind, InlineKeyboardButton, InlineKeyboardMarkup, InlineQueryResult, InlineQueryResultArticle, InputFile, InputMedia,
        InputMediaAudio, InputMediaPhoto, InputMediaVideo, InputTextMessageContent, LinkPreviewOptions, Message, ReplyParameters,
    },
    Bot,
};
use tokio::{sync::Mutex, time::Instant};
use tracing::{error, warn};

const UPLOAD_TIMEOUT: Duration = Duration::from_secs(proto::MEDIA_EXECUTION_LIMIT_SECS);
const PROGRESS_EDIT_INTERVAL: Duration = Duration::from_secs(5);
const INLINE_PROGRESS_EDIT_INTERVAL: Duration = Duration::from_secs(15);
const WAIT_LIVENESS_INTERVAL: Duration = Duration::from_secs(30);
const REQUEST_INTERVAL: Duration = Duration::from_millis(100);

use super::{
    AnswerGuestRequest, AnswerInlineErrorRequest, AnswerInlineQueryRequest, DeleteMessageRequest, EditMediaByIdRequest, EditTarget,
    EditTextRequest, InlineQueryArticle, MessengerError, SendMediaByIdRequest, SendMediaGroupRequest, SendTextRequest, SentMessage,
    TextFormat, UploadAudioRequest, UploadPhotoRequest, UploadPhotoUrlRequest, UploadVideoRequest,
};

#[derive(Clone)]
pub struct TelegramMessenger {
    bot: Arc<Bot>,
    error_formatter: Arc<ErrorFormatter>,
    timeouts_cfg: Arc<TimeoutsConfig>,
    limits: Arc<Mutex<RequestLimits>>,
    progress_throttle: Arc<ProgressThrottle>,
}

struct RequestLimits {
    cooldown_until: Instant,
    next_request: Instant,
}

impl Default for RequestLimits {
    fn default() -> Self {
        Self {
            cooldown_until: Instant::now(),
            next_request: Instant::now(),
        }
    }
}

impl TelegramMessenger {
    pub fn new(
        bot: Arc<Bot>,
        error_formatter: Arc<ErrorFormatter>,
        timeouts_cfg: Arc<TimeoutsConfig>,
        progress_throttle: Arc<ProgressThrottle>,
    ) -> Self {
        Self {
            bot,
            error_formatter,
            timeouts_cfg,
            limits: Arc::new(Mutex::new(RequestLimits::default())),
            progress_throttle,
        }
    }

    async fn allow_progress(&self, target: &EditTarget<'_>) -> bool {
        {
            let limits = self.limits.lock().await;
            if limits.cooldown_until.max(limits.next_request) > Instant::now() {
                return false;
            }
        }
        let (target, interval) = match target {
            EditTarget::ChatMessage { chat_id, .. } => (format!("chat:{chat_id}"), PROGRESS_EDIT_INTERVAL),
            EditTarget::InlineMessage { inline_message_id } => (format!("inline:{inline_message_id}"), INLINE_PROGRESS_EDIT_INTERVAL),
        };
        let key = format!("telegram:{}:progress:{target}", self.bot.id);
        match tokio::time::timeout(Duration::from_secs(1), self.progress_throttle.try_acquire(&key, interval)).await {
            Ok(Ok(allowed)) => allowed,
            Ok(Err(err)) => {
                warn!(%err, "Redis progress throttling failed; skipping update");
                false
            }
            Err(err) => {
                warn!(%err, "Redis progress throttling timed out; skipping update");
                false
            }
        }
    }
}

impl From<SessionErrorKind> for MessengerError {
    fn from(value: SessionErrorKind) -> Self {
        Self::with_category(value.to_string(), telegram_error_category(&value))
    }
}

fn telegram_error_category(error: &SessionErrorKind) -> &'static str {
    match error {
        SessionErrorKind::Client(_) => "telegram_transport",
        SessionErrorKind::Parse(_) => "telegram_response_decode",
        SessionErrorKind::Telegram(error) => match error {
            TelegramErrorKind::NetworkError { .. } => "telegram_transport",
            TelegramErrorKind::RetryAfter { .. } => "telegram_rate_limit",
            TelegramErrorKind::MigrateToChat { .. } => "telegram_chat_migrated",
            TelegramErrorKind::BadRequest { message } => {
                let message = message.to_ascii_lowercase();
                if message.contains("query is too old") || message.contains("query_id_invalid") || message.contains("query id is invalid") {
                    "telegram_query_expired_or_invalid"
                } else {
                    "telegram_bad_request"
                }
            }
            TelegramErrorKind::NotFound { .. } => "telegram_not_found",
            TelegramErrorKind::ConflictError { .. } => "telegram_conflict",
            TelegramErrorKind::Forbidden { .. } => "telegram_forbidden",
            TelegramErrorKind::Unauthorized { .. } => "telegram_unauthorized",
            TelegramErrorKind::ServerError { .. } => "telegram_server",
            TelegramErrorKind::RestartingTelegram { .. } => "telegram_restarting",
            TelegramErrorKind::EntityTooLarge { .. } => "telegram_entity_too_large",
            TelegramErrorKind::UnknownError(_) => "telegram_unknown",
        },
    }
}

impl MessengerPort for TelegramMessenger {
    async fn answer_guest(&self, request: AnswerGuestRequest<'_>) -> Result<String, MessengerError> {
        let result = InlineQueryResultArticle::new(
            "guest",
            request.text,
            InputTextMessageContent::new(request.text).link_preview_options(LinkPreviewOptions::new().is_disabled(true)),
        );
        // A transport error may mean the single reply was sent. Never replay that request.
        // Explicit flood waits are handled by the shared cooldown as for other text requests.
        with_retries(self, methods::AnswerGuestQuery::new(request.query_id, result), 0, Some(30.0))
            .await
            .map(|sent| sent.inline_message_id.into())
            .map_err(|err| MessengerError::with_category("Could not answer guest query", telegram_error_category(&err)))
    }

    async fn username(&self) -> Result<String, MessengerError> {
        let me = with_retries(self, GetMe {}, 0, None).await?;
        Ok(me.username.expect("Bots always have a username").into())
    }

    async fn send_text(&self, request: SendTextRequest<'_>) -> Result<SentMessage, MessengerError> {
        let message = with_retries(
            self,
            SendMessage::new(request.chat_id, request.text)
                .parse_mode_option(request.format.map(ParseMode::from))
                .link_preview_options(LinkPreviewOptions::new().is_disabled(request.disable_link_preview))
                .reply_parameters_option(
                    request
                        .reply_to_message_id
                        .map(|id| ReplyParameters::new().message_id(id).allow_sending_without_reply(true)),
                ),
            0,
            None,
        )
        .await?;
        Ok(SentMessage {
            message_id: message.message_id(),
        })
    }

    async fn edit_text(&self, request: EditTextRequest<'_>) -> Result<(), MessengerError> {
        if request.is_progress {
            if !self.allow_progress(&request.target).await {
                return Ok(());
            }
            let mut limits = self.limits.lock().await;
            if limits.cooldown_until.max(limits.next_request) > Instant::now() {
                return Ok(());
            }
            limits.next_request = Instant::now() + REQUEST_INTERVAL;
        }
        let method = EditMessageText::new()
            .text(request.text)
            .parse_mode_option(request.format.map(ParseMode::from))
            .link_preview_options(LinkPreviewOptions::new().is_disabled(request.disable_link_preview));

        let method = match request.target {
            EditTarget::ChatMessage { chat_id, message_id } => method.chat_id(chat_id).message_id(message_id),
            EditTarget::InlineMessage { inline_message_id } => {
                let method = method.inline_message_id(inline_message_id);
                if request.clear_inline_keyboard {
                    method.reply_markup(InlineKeyboardMarkup::new([[]]))
                } else {
                    method
                }
            }
        };
        if request.is_progress {
            let result = perform_request(self, method, None).await;
            if !result.as_ref().err().is_some_and(|err| telegram_retry_after(err).is_some()) {
                result?;
            }
        } else {
            with_retries(self, method, 0, None).await?;
        }
        Ok(())
    }

    async fn delete_message(&self, request: DeleteMessageRequest) -> Result<(), MessengerError> {
        with_retries(self, DeleteMessage::new(request.chat_id, request.message_id), 0, None).await?;
        Ok(())
    }

    async fn answer_inline_error(&self, request: AnswerInlineErrorRequest<'_>) -> Result<(), MessengerError> {
        let result = InlineQueryResultArticle::new(request.query_id, request.text, InputTextMessageContent::new(request.text));

        once(self, AnswerInlineQuery::new(request.query_id, [result]).cache_time(0), None).await?;
        Ok(())
    }

    async fn answer_inline_query(&self, request: AnswerInlineQueryRequest<'_>) -> Result<(), MessengerError> {
        let results: Vec<InlineQueryResult> = request.results.into_iter().map(InlineQueryResult::from).collect();

        once(
            self,
            AnswerInlineQuery::new(request.query_id, results)
                .cache_time(request.cache_time)
                .is_personal(request.is_personal),
            None,
        )
        .await?;
        Ok(())
    }

    async fn upload_video(&self, request: UploadVideoRequest<'_>) -> Result<Box<str>, MessengerError> {
        let UploadVideoRequest {
            chat_id,
            reply_to_message_id,
            media_for_upload:
                MediaForUpload {
                    path,
                    thumb_stream,
                    temp_dir,
                    stream,
                    deadline,
                },
            name,
            width,
            height,
            duration,
            with_delete,
            webpage_url,
            link_is_visible,
        } = request;
        let send_name = sanitize_send_filename(path.as_ref(), name);
        let video = InputFile::stream_with_name(upload_stream(stream), &send_name);
        let thumbnail = thumb_stream.map(|stream| InputFile::stream_with_name(stream.into_inner(), "thumbnail.jpg"));
        let method = methods::SendVideo::new(chat_id, video)
            .width_option(width)
            .height_option(height)
            .supports_streaming(true)
            .duration_option(duration)
            .disable_notification(true)
            .thumbnail_option(thumbnail)
            .caption_option(if link_is_visible { media_link(Some(webpage_url)) } else { None })
            .parse_mode(ParseMode::HTML)
            .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new().message_id(id).allow_sending_without_reply(true)));

        let message = until_upload_deadline(once(self, method, Some(UPLOAD_TIMEOUT.as_secs_f32())), deadline).await?;
        drop(temp_dir);
        let message_id = message.message_id();
        let file_id = match message.video() {
            Some(video) => video.file_id.clone(),
            None => message.document().expect("Video upload returns video or document").file_id.clone(),
        };
        drop(message);

        if with_delete {
            self.spawn_delete_message(chat_id, message_id);
        }

        Ok(file_id)
    }

    async fn upload_audio(&self, request: UploadAudioRequest<'_>) -> Result<Box<str>, MessengerError> {
        let UploadAudioRequest {
            chat_id,
            reply_to_message_id,
            media_for_upload:
                MediaForUpload {
                    path,
                    thumb_stream,
                    temp_dir,
                    stream,
                    deadline,
                },
            name,
            title,
            performer,
            duration,
            with_delete,
            webpage_url,
            link_is_visible,
        } = request;
        let send_name = sanitize_send_filename(path.as_ref(), name);
        let audio = InputFile::stream_with_name(upload_stream(stream), &send_name);
        let thumbnail = thumb_stream.map(|stream| InputFile::stream_with_name(stream.into_inner(), "thumbnail.jpg"));
        let method = methods::SendAudio::new(chat_id, audio)
            .title_option(title)
            .duration_option(duration)
            .disable_notification(true)
            .performer_option(performer)
            .thumbnail_option(thumbnail)
            .caption_option(if link_is_visible { media_link(Some(webpage_url)) } else { None })
            .parse_mode(ParseMode::HTML)
            .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new().message_id(id).allow_sending_without_reply(true)));

        let message = until_upload_deadline(once(self, method, Some(UPLOAD_TIMEOUT.as_secs_f32())), deadline).await?;
        drop(temp_dir);
        let message_id = message.message_id();
        let file_id = message
            .audio()
            .map(|val| val.file_id.clone())
            .or(message.voice().map(|val| val.file_id.clone()))
            .expect("Audio upload returns audio or voice");
        drop(message);

        if with_delete {
            self.spawn_delete_message(chat_id, message_id);
        }

        Ok(file_id)
    }

    async fn upload_photo(&self, request: UploadPhotoRequest<'_>) -> Result<Box<str>, MessengerError> {
        let UploadPhotoRequest {
            chat_id,
            reply_to_message_id,
            media_for_upload:
                MediaForUpload {
                    path,
                    temp_dir,
                    stream,
                    deadline,
                    ..
                },
            name,
            with_delete,
            webpage_url,
            link_is_visible,
        } = request;
        let send_name = sanitize_send_filename(path.as_ref(), name);
        let photo = InputFile::stream_with_name(upload_stream(stream), &send_name);
        let method = methods::SendPhoto::new(chat_id, photo)
            .disable_notification(true)
            .caption_option(if link_is_visible { media_link(Some(webpage_url)) } else { None })
            .parse_mode(ParseMode::HTML)
            .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new().message_id(id).allow_sending_without_reply(true)));

        let message = until_upload_deadline(once(self, method, Some(UPLOAD_TIMEOUT.as_secs_f32())), deadline).await?;
        drop(temp_dir);
        let message_id = message.message_id();
        let file_id = message
            .photo()
            .and_then(|photos| photos.last())
            .map(|photo| photo.file_id.clone())
            .or(message.document().map(|document| document.file_id.clone()))
            .expect("Photo upload returns photo or document");
        drop(message);

        if with_delete {
            self.spawn_delete_message(chat_id, message_id);
        }

        Ok(file_id)
    }

    async fn upload_photo_url(&self, request: UploadPhotoUrlRequest<'_>) -> Result<Box<str>, MessengerError> {
        let UploadPhotoUrlRequest {
            chat_id,
            reply_to_message_id,
            photo_url,
            with_delete,
            webpage_url,
            link_is_visible,
        } = request;
        let method = methods::SendPhoto::new(chat_id, InputFile::url(photo_url.as_str()))
            .disable_notification(true)
            .caption_option(if link_is_visible { media_link(Some(webpage_url)) } else { None })
            .parse_mode(ParseMode::HTML)
            .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new().message_id(id).allow_sending_without_reply(true)));

        let message = once(self, method, Some(UPLOAD_TIMEOUT.as_secs_f32())).await?;
        let message_id = message.message_id();
        let file_id = message
            .photo()
            .and_then(|photos| photos.last())
            .map(|photo| photo.file_id.clone())
            .expect("Photo URL upload returns photo");
        drop(message);

        if with_delete {
            self.spawn_delete_message(chat_id, message_id);
        }

        Ok(file_id)
    }

    async fn send_video_by_id(&self, request: SendMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        with_retries(
            self,
            methods::SendVideo::new(request.chat_id, InputFile::id(request.remote_id))
                .reply_parameters_option(
                    request
                        .reply_to_message_id
                        .map(|id| ReplyParameters::new().message_id(id).allow_sending_without_reply(true)),
                )
                .caption_option(if request.link_is_visible {
                    media_link(request.webpage_url)
                } else {
                    None
                })
                .disable_notification(true)
                .supports_streaming(true)
                .parse_mode(ParseMode::HTML),
            2,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        Ok(())
    }

    async fn send_audio_by_id(&self, request: SendMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        with_retries(
            self,
            methods::SendAudio::new(request.chat_id, InputFile::id(request.remote_id))
                .reply_parameters_option(
                    request
                        .reply_to_message_id
                        .map(|id| ReplyParameters::new().message_id(id).allow_sending_without_reply(true)),
                )
                .caption_option(caption_with_link(
                    request.caption.map(ToOwned::to_owned),
                    request.link_is_visible,
                    request.webpage_url,
                ))
                .disable_notification(true)
                .parse_mode(ParseMode::HTML),
            2,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        Ok(())
    }

    async fn send_photo_by_id(&self, request: SendMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        with_retries(
            self,
            methods::SendPhoto::new(request.chat_id, InputFile::id(request.remote_id))
                .reply_parameters_option(
                    request
                        .reply_to_message_id
                        .map(|id| ReplyParameters::new().message_id(id).allow_sending_without_reply(true)),
                )
                .caption_option(if request.link_is_visible {
                    media_link(request.webpage_url)
                } else {
                    None
                })
                .disable_notification(true)
                .parse_mode(ParseMode::HTML),
            2,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        Ok(())
    }

    async fn edit_video_by_id(&self, request: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        with_retries(
            self,
            methods::EditMessageMedia::new(
                InputMediaVideo::new(InputFile::id(request.remote_id))
                    .caption_option(if request.link_is_visible {
                        media_link(request.webpage_url)
                    } else {
                        None
                    })
                    .supports_streaming(true)
                    .parse_mode(ParseMode::HTML),
            )
            .inline_message_id(request.inline_message_id)
            .reply_markup(InlineKeyboardMarkup::new([[]])),
            2,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        Ok(())
    }

    async fn edit_audio_by_id(&self, request: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        with_retries(
            self,
            methods::EditMessageMedia::new(
                InputMediaAudio::new(InputFile::id(request.remote_id))
                    .caption_option(if request.link_is_visible {
                        media_link(request.webpage_url)
                    } else {
                        None
                    })
                    .parse_mode(ParseMode::HTML),
            )
            .inline_message_id(request.inline_message_id)
            .reply_markup(InlineKeyboardMarkup::new([[]])),
            2,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        Ok(())
    }

    async fn edit_photo_by_id(&self, request: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        with_retries(
            self,
            methods::EditMessageMedia::new(
                InputMediaPhoto::new(InputFile::id(request.remote_id))
                    .caption_option(if request.link_is_visible {
                        media_link(request.webpage_url)
                    } else {
                        None
                    })
                    .parse_mode(ParseMode::HTML),
            )
            .inline_message_id(request.inline_message_id)
            .reply_markup(InlineKeyboardMarkup::new([[]])),
            2,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        Ok(())
    }

    async fn send_video_group(&self, request: SendMediaGroupRequest) -> Result<(), MessengerError> {
        if let [item] = request.items.as_slice() {
            return self
                .send_video_by_id(SendMediaByIdRequest {
                    chat_id: request.chat_id,
                    reply_to_message_id: request.reply_to_message_id,
                    remote_id: &item.remote_id,
                    webpage_url: item.webpage_url.as_ref(),
                    link_is_visible: request.link_is_visible,
                    caption: request.caption.as_deref(),
                })
                .await;
        }
        media_groups(
            self,
            request.chat_id,
            request
                .items
                .into_iter()
                .map(|item| {
                    InputMediaVideo::new(InputFile::id(item.remote_id))
                        .caption_option(if request.link_is_visible {
                            media_link(item.webpage_url.as_ref())
                        } else {
                            None
                        })
                        .parse_mode(ParseMode::HTML)
                })
                .collect(),
            request.reply_to_message_id,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        Ok(())
    }

    async fn send_audio_group(
        &self,
        SendMediaGroupRequest {
            chat_id,
            reply_to_message_id,
            items,
            link_is_visible,
            caption,
        }: SendMediaGroupRequest,
    ) -> Result<(), MessengerError> {
        if let [item] = items.as_slice() {
            return self
                .send_audio_by_id(SendMediaByIdRequest {
                    chat_id,
                    reply_to_message_id,
                    remote_id: &item.remote_id,
                    webpage_url: item.webpage_url.as_ref(),
                    link_is_visible,
                    caption: caption.as_deref(),
                })
                .await;
        }
        media_groups(
            self,
            chat_id,
            items
                .into_iter()
                .map(|item| {
                    let item_caption = caption_with_link(caption.clone(), link_is_visible, item.webpage_url.as_ref());
                    InputMediaAudio::new(InputFile::id(item.remote_id))
                        .caption_option(item_caption)
                        .parse_mode(ParseMode::HTML)
                })
                .collect(),
            reply_to_message_id,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        Ok(())
    }

    async fn send_photo_group(&self, request: SendMediaGroupRequest) -> Result<(), MessengerError> {
        if let [item] = request.items.as_slice() {
            return self
                .send_photo_by_id(SendMediaByIdRequest {
                    chat_id: request.chat_id,
                    reply_to_message_id: request.reply_to_message_id,
                    remote_id: &item.remote_id,
                    webpage_url: item.webpage_url.as_ref(),
                    link_is_visible: request.link_is_visible,
                    caption: request.caption.as_deref(),
                })
                .await;
        }
        media_groups(
            self,
            request.chat_id,
            request
                .items
                .into_iter()
                .map(|item| {
                    InputMediaPhoto::new(InputFile::id(item.remote_id))
                        .caption_option(if request.link_is_visible {
                            media_link(item.webpage_url.as_ref())
                        } else {
                            None
                        })
                        .parse_mode(ParseMode::HTML)
                })
                .collect(),
            request.reply_to_message_id,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        Ok(())
    }
}

impl TelegramMessenger {
    /// Best-effort fire-and-forget delete that logs failures via `error_formatter`.
    /// Used after every "send + auto-delete" upload (the receiver chat is just a
    /// staging area whose messages get deleted once we have the `file_id`).
    fn spawn_delete_message(&self, chat_id: i64, message_id: i64) {
        let messenger = self.clone();
        let error_formatter = self.error_formatter.clone();
        tokio::spawn(async move {
            if let Err(err) = with_retries(&messenger, methods::DeleteMessage::new(chat_id, message_id), 0, None).await {
                let err = MessengerError::from(err);
                error!(err = %error_formatter.format(&err), "Delete message error");
            }
        });
    }
}

impl From<TextFormat> for ParseMode {
    fn from(value: TextFormat) -> Self {
        match value {
            TextFormat::Html => ParseMode::HTML,
        }
    }
}

impl From<InlineQueryArticle> for InlineQueryResult {
    fn from(article: InlineQueryArticle) -> Self {
        let mut result = InlineQueryResultArticle::new(
            article.id,
            article.title,
            InputTextMessageContent::new(article.content_text).parse_mode_option(article.content_format.map(ParseMode::from)),
        );

        if let Some(thumbnail_url) = article.thumbnail_url {
            result = result.thumbnail_url(thumbnail_url);
        }
        if let Some(description) = article.description {
            result = result.description(description);
        }
        if let Some(callback_data) = article.callback_data {
            result = result.reply_markup(InlineKeyboardMarkup::new([[
                InlineKeyboardButton::new("...").callback_data(callback_data)
            ]]));
        }

        result.into()
    }
}

/// Builds a media caption: an optional custom caption (e.g. recognized-song metadata) followed by
/// the source "Link" when visible. Either part may be absent.
fn caption_with_link(caption: Option<String>, link_is_visible: bool, webpage_url: Option<&url::Url>) -> Option<String> {
    let link = if link_is_visible { media_link(webpage_url) } else { None };
    match (caption, link) {
        (Some(caption), Some(link)) => Some(format!("{caption}\n\n{link}")),
        (Some(text), None) | (None, Some(text)) => Some(text),
        (None, None) => None,
    }
}

#[allow(clippy::cast_sign_loss)]
async fn once<T>(messenger: &TelegramMessenger, method: T, request_timeout: Option<f32>) -> Result<T::Return, SessionErrorKind>
where
    T: TelegramMethod + Send + Sync,
    T::Method: Send + Sync,
{
    wait_for_request(&messenger.limits).await;
    perform_request(messenger, method, request_timeout).await
}

async fn wait_for_request(limits: &Mutex<RequestLimits>) {
    loop {
        let mut limits = limits.lock().await;
        let ready = limits.cooldown_until.max(limits.next_request);
        if ready <= Instant::now() {
            limits.next_request = Instant::now() + REQUEST_INTERVAL;
            break;
        }
        drop(limits);
        // Intentional Telegram waiting keeps the queue delivery alive, not its media deadline.
        record_execution_progress();
        tokio::time::sleep_until(ready.min(Instant::now() + WAIT_LIVENESS_INTERVAL)).await;
    }
}

async fn perform_request<T>(messenger: &TelegramMessenger, method: T, request_timeout: Option<f32>) -> Result<T::Return, SessionErrorKind>
where
    T: TelegramMethod + Send + Sync,
    T::Method: Send + Sync,
{
    let result = if let Some(request_timeout) = request_timeout {
        messenger.bot.send_with_timeout(method, request_timeout).await
    } else {
        messenger.bot.send(method).await
    };
    if let Some(delay) = result.as_ref().err().and_then(telegram_retry_after) {
        let mut limits = messenger.limits.lock().await;
        limits.cooldown_until = limits.cooldown_until.max(Instant::now() + delay);
    }
    if result.is_ok() {
        record_execution_progress();
    }
    result
}

fn telegram_retry_after(error: &SessionErrorKind) -> Option<Duration> {
    let seconds = match error {
        SessionErrorKind::Telegram(TelegramErrorKind::RetryAfter { retry_after, .. }) => u32::try_from(*retry_after).ok()?,
        SessionErrorKind::Telegram(TelegramErrorKind::BadRequest { message }) => {
            // The local Bot API can return a flood wait as HTTP 400 without response parameters.
            let message = message.trim().to_ascii_lowercase();
            let message = message.strip_prefix("bad request: ").unwrap_or(&message);
            message.strip_prefix("too many requests: retry after ")?.parse::<u32>().ok()?
        }
        _ => return None,
    };
    (seconds > 0).then(|| Duration::from_secs(u64::from(seconds)))
}

fn upload_stream(stream: MediaByteStream) -> impl Stream<Item = Result<Bytes, io::Error>> + Send + Sync + Unpin {
    let execution = current_execution_outcome();
    stream.into_inner().inspect(move |item| {
        if matches!(item, Ok(bytes) if !bytes.is_empty()) {
            if let Some(outcome) = &execution {
                outcome.record_progress();
            }
        }
    })
}

async fn until_upload_deadline<T>(
    upload: impl Future<Output = Result<T, SessionErrorKind>>,
    deadline: Instant,
) -> Result<T, MessengerError> {
    tokio::time::timeout_at(deadline, upload)
        .await
        .map_err(|_| MessengerError::with_category("Media execution exceeded the per-media time limit", "media_timeout"))?
        .map_err(Into::into)
}

#[allow(clippy::cast_sign_loss)]
async fn with_retries<T>(
    messenger: &TelegramMessenger,
    method: T,
    max_retries: u8,
    request_timeout: Option<f32>,
) -> Result<T::Return, SessionErrorKind>
where
    T: TelegramMethod + Clone + Send + Sync,
    T::Method: Send + Sync,
{
    let cur_retry_count = AtomicU8::new(0);

    backoff::future::retry(ExponentialBackoff::default(), || async {
        match once(messenger, method.clone(), request_timeout).await {
            Ok(res) => Ok(res),
            Err(err) if telegram_retry_after(&err).is_some() => {
                warn!(category = telegram_error_category(&err), "Waiting for Telegram cooldown");
                // The shared request gate owns this wait and records queue liveness.
                Err(backoff::Error::retry_after(err, Duration::ZERO))
            }
            Err(err) => Err(match err {
                SessionErrorKind::Telegram(TelegramErrorKind::ServerError { .. } | TelegramErrorKind::MigrateToChat { .. }) => {
                    cur_retry_count.fetch_add(1, Relaxed);
                    if cur_retry_count.load(Relaxed) > max_retries {
                        backoff::Error::permanent(err)
                    } else {
                        backoff::Error::transient(err)
                    }
                }
                _ => backoff::Error::permanent(err),
            }),
        }
    })
    .await
}

/// Telegram media groups are limited to 10 items; continue past failed batches,
/// but report a partial send to the caller after processing the list.
async fn media_groups(
    messenger: &TelegramMessenger,
    chat_id: impl Into<ChatIdKind>,
    input_media_list: Vec<impl Into<InputMedia>>,
    reply_to_message_id: Option<i64>,
    request_timeout: Option<f32>,
) -> Result<Box<[Message]>, MessengerError> {
    const MAX_MEDIA_GROUP: usize = 10;

    let chat_id = chat_id.into();
    let input_media_len = input_media_list.len();

    if input_media_len == 0 {
        return Ok(Box::new([]));
    }

    let mut messages = Vec::with_capacity(input_media_len);
    let mut remaining = input_media_list.into_iter().map(Into::into).collect::<Vec<InputMedia>>();
    let mut last_error = None;

    while !remaining.is_empty() {
        // Keep two items for the last group instead of sending an invalid singleton.
        let group_len = if remaining.len() == MAX_MEDIA_GROUP + 1 {
            MAX_MEDIA_GROUP - 1
        } else {
            remaining.len().min(MAX_MEDIA_GROUP)
        };
        if let Err(err) = send_media_group(
            messenger,
            &chat_id,
            remaining.drain(..group_len).collect(),
            reply_to_message_id,
            request_timeout,
            &mut messages,
        )
        .await
        {
            last_error = Some(err);
        }
    }

    ensure_all_media_sent(messages.len(), input_media_len, last_error)?;

    Ok(messages.into())
}

fn ensure_all_media_sent(sent: usize, total: usize, last_error: Option<SessionErrorKind>) -> Result<(), MessengerError> {
    if let Some(err) = last_error {
        return Err(MessengerError::new(format!(
            "Sent {sent} of {total} media items; last error: {err}"
        )));
    }
    Ok(())
}

async fn send_media_group(
    messenger: &TelegramMessenger,
    chat_id: &ChatIdKind,
    media_group: Vec<InputMedia>,
    reply_to_message_id: Option<i64>,
    request_timeout: Option<f32>,
    messages: &mut Vec<Message>,
) -> Result<(), SessionErrorKind> {
    let media_group_len = media_group.len();
    let res = with_retries(
        messenger,
        SendMediaGroup::new(chat_id.clone(), media_group)
            .disable_notification(true)
            .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new().message_id(id).allow_sending_without_reply(true))),
        3,
        request_timeout,
    )
    .await;
    match res {
        Ok(new_messages) => {
            messages.extend(new_messages);
            Ok(())
        }
        Err(err) => {
            warn!("Skip {media_group_len} media count to send");
            Err(err)
        }
    }
}

#[cfg(test)]
mod upload_deadline_tests {
    use super::*;
    use futures_util::stream;

    #[tokio::test(start_paused = true)]
    async fn each_playlist_upload_has_a_fresh_hard_budget() {
        let started = Instant::now();
        for _ in 0..10 {
            let upload = async {
                tokio::time::sleep(Duration::from_secs(8)).await;
                Ok::<_, SessionErrorKind>(())
            };
            until_upload_deadline(upload, Instant::now() + Duration::from_secs(10))
                .await
                .unwrap();
        }
        assert_eq!(started.elapsed(), Duration::from_secs(80));
    }

    #[tokio::test(start_paused = true)]
    async fn progress_does_not_extend_an_upload_deadline() {
        let upload = async {
            for _ in 0..20 {
                tokio::time::sleep(Duration::from_millis(50)).await;
                record_execution_progress();
            }
            std::future::pending::<Result<(), SessionErrorKind>>().await
        };
        let started = Instant::now();
        let error = until_upload_deadline(upload, started + Duration::from_millis(500))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("time limit"));
        assert_eq!(started.elapsed(), Duration::from_millis(500));
    }

    #[tokio::test]
    async fn consumed_upload_bytes_report_progress_even_on_another_task() {
        let outcome = downloader_client::outcome::ExecutionOutcome::default();
        let mut updates = outcome.subscribe_progress();
        let upload = outcome
            .scope(async { upload_stream(MediaByteStream::new(stream::iter([Ok(Bytes::from_static(b"media"))]))) })
            .await;
        let bytes = tokio::spawn(async move { upload.collect::<Vec<_>>().await }).await.unwrap();
        assert_eq!(bytes.len(), 1);
        tokio::time::timeout(Duration::from_secs(1), updates.changed())
            .await
            .unwrap()
            .unwrap();
    }
}

#[cfg(test)]
mod rate_limit_tests {
    use super::*;
    use crate::services::{messenger::MediaGroupItem, queue::test_support::Fixture};
    use futures_util::stream;
    use std::{borrow::Cow, net::SocketAddr};
    use telers::client::{
        telegram::{APIServer, BareFilesPathWrapper},
        Reqwest,
    };
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::{TcpListener, TcpStream},
    };
    use url::Url;

    #[test]
    fn guest_error_categories_distinguish_transport_and_response_failures() {
        let transport = SessionErrorKind::Client(io::Error::other("Synthetic private request URL").into());
        let decoding = SessionErrorKind::Parse(serde_json::from_str::<()>("synthetic invalid JSON").unwrap_err());
        assert_eq!(telegram_error_category(&transport), "telegram_transport");
        assert_eq!(telegram_error_category(&decoding), "telegram_response_decode");
    }

    #[tokio::test]
    async fn guest_reply_errors_preserve_categories_without_private_payloads_or_retries() {
        let fixture = Fixture::new().await;
        for (code, description, category) in [
            (400, "Bad Request: query is too old", "telegram_query_expired_or_invalid"),
            (404, "Not Found", "telegram_not_found"),
            (403, "Forbidden", "telegram_forbidden"),
            (500, "Internal Server Error", "telegram_server"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_http_request(&mut socket).await;
                assert!(String::from_utf8_lossy(&request).contains("/answerGuestQuery "));
                let body = serde_json::json!({
                    "ok": false, "error_code": code,
                    "description": format!("{description}: https://example.test/private?secret=synthetic"),
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 {code} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                assert!(tokio::time::timeout(Duration::from_millis(200), listener.accept()).await.is_err());
            });
            let messenger = test_messenger(address, &fixture);
            let error = tokio::time::timeout(
                Duration::from_secs(2),
                messenger.answer_guest(AnswerGuestRequest {
                    query_id: "synthetic-query",
                    text: "Preparing",
                }),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert_eq!(error.category(), category);
            let diagnostic = format!("{error:?}");
            assert!(!diagnostic.contains("secret="));
            assert!(!diagnostic.contains("example.test"));
            server.await.unwrap();
        }
    }

    #[test]
    fn partial_media_group_failure_is_reported() {
        let failure = SessionErrorKind::Telegram(TelegramErrorKind::BadRequest {
            message: "Synthetic rejection".into(),
        });
        let error = ensure_all_media_sent(10, 12, Some(failure)).unwrap_err();
        assert!(error.to_string().contains("Sent 10 of 12 media items"));
        assert!(error.to_string().contains("Synthetic rejection"));
        ensure_all_media_sent(12, 12, None).unwrap();
    }

    #[tokio::test]
    async fn media_groups_report_a_failed_batch_without_sending_a_singleton_group() {
        let fixture = Fixture::new().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut group_sizes = Vec::new();
            for index in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_http_request(&mut socket).await;
                let media_start = request
                    .windows(b"name=\"media\"\r\n\r\n".len())
                    .position(|bytes| bytes == b"name=\"media\"\r\n\r\n")
                    .unwrap()
                    + b"name=\"media\"\r\n\r\n".len();
                let media_end = request[media_start..].windows(4).position(|bytes| bytes == b"\r\n--").unwrap() + media_start;
                let media: Vec<serde_json::Value> = serde_json::from_slice(&request[media_start..media_end]).unwrap();
                let size = media.len();
                group_sizes.push(size);
                let (status, body) = if index == 0 {
                    let messages = (0..size)
                        .map(|id| serde_json::json!({"message_id": id + 1, "date": 1, "chat": {"id": 1, "type": "private"}}))
                        .collect::<Vec<_>>();
                    ("200 OK", serde_json::json!({"ok": true, "result": messages}).to_string())
                } else {
                    (
                        "400 Bad Request",
                        serde_json::json!({"ok": false, "error_code": 400, "description": "Synthetic group rejection"}).to_string(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            group_sizes
        });
        let messenger = test_messenger(address, &fixture);
        let items = (0..11)
            .map(|index| InputMediaPhoto::new(InputFile::id(format!("synthetic-{index}"))))
            .collect();
        let error = media_groups(&messenger, 1, items, None, Some(5.0)).await.unwrap_err();
        assert!(error.to_string().contains("Sent 9 of 11 media items"));
        assert_eq!(server.await.unwrap(), vec![9, 2]);
    }

    #[tokio::test]
    async fn one_item_playlist_uses_the_existing_file_id_send() {
        let fixture = Fixture::new().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            let first_line = String::from_utf8_lossy(&request).lines().next().unwrap().to_owned();
            let body = r#"{"ok":true,"result":{"message_id":1,"date":1,"chat":{"id":1,"type":"private"}}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            first_line
        });
        let messenger = test_messenger(address, &fixture);
        messenger
            .send_photo_group(SendMediaGroupRequest {
                chat_id: 1,
                reply_to_message_id: None,
                items: vec![MediaGroupItem {
                    remote_id: "synthetic-photo".into(),
                    webpage_url: None,
                }],
                link_is_visible: false,
                caption: None,
            })
            .await
            .unwrap();
        assert!(server.await.unwrap().contains("/sendPhoto "));
    }

    async fn read_http_request(socket: &mut TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = socket.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&buffer[..count]);
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    return request;
                }
            }
        }
    }

    fn test_messenger(address: SocketAddr, fixture: &Fixture) -> TelegramMessenger {
        let api = APIServer::new(
            &format!("http://{address}/bot{{token}}/{{method_name}}"),
            &format!("http://{address}/files/{{path}}"),
            true,
            BareFilesPathWrapper,
        );
        TelegramMessenger::new(
            Arc::new(Bot::with_client(
                "123:synthetic",
                Reqwest::default().with_api_server(Cow::Owned(api)),
            )),
            Arc::new(ErrorFormatter::new("synthetic")),
            Arc::new(TimeoutsConfig::default()),
            Arc::new(ProgressThrottle::new(fixture.connection())),
        )
    }

    #[tokio::test]
    async fn telegram_cooldown_is_shared_and_final_error_edits_are_not_dropped() {
        assert_shared_cooldown(
            "429 Too Many Requests",
            r#"{"ok":false,"error_code":429,"description":"Too Many Requests","parameters":{"retry_after":2}}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn text_form_flood_wait_also_sets_the_shared_cooldown() {
        assert_shared_cooldown(
            "400 Bad Request",
            r#"{"ok":false,"error_code":400,"description":"Bad Request: too Many Requests: retry after 2"}"#,
        )
        .await;
    }

    async fn assert_shared_cooldown(error_status: &'static str, error_body: &'static str) {
        let fixture = Fixture::new().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut received = Vec::new();
            for index in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let _request = read_http_request(&mut socket).await;
                received.push(Instant::now());
                let (status, body) = if index == 0 {
                    (error_status, error_body)
                } else {
                    ("200 OK", r#"{"ok":true,"result":true}"#)
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            received
        });
        let messenger = test_messenger(address, &fixture);
        messenger
            .edit_text(EditTextRequest {
                is_progress: true,
                target: EditTarget::ChatMessage { chat_id: 1, message_id: 1 },
                text: "Downloading",
                format: None,
                disable_link_preview: true,
                clear_inline_keyboard: false,
            })
            .await
            .unwrap();
        // This request must be discarded rather than sent during the shared cooldown.
        messenger
            .clone()
            .edit_text(EditTextRequest {
                is_progress: true,
                target: EditTarget::ChatMessage { chat_id: 2, message_id: 2 },
                text: "Downloading",
                format: None,
                disable_link_preview: true,
                clear_inline_keyboard: false,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            let (edit, delete) = tokio::join!(
                messenger.edit_text(EditTextRequest {
                    is_progress: false,
                    target: EditTarget::ChatMessage { chat_id: 1, message_id: 1 },
                    text: "Download failed",
                    format: None,
                    disable_link_preview: true,
                    clear_inline_keyboard: false,
                }),
                messenger.delete_message(DeleteMessageRequest { chat_id: 2, message_id: 2 }),
            );
            edit.unwrap();
            delete.unwrap();
        })
        .await
        .unwrap();
        let received = tokio::time::timeout(Duration::from_secs(1), server).await.unwrap().unwrap();
        assert!(received[1].duration_since(received[0]) >= Duration::from_secs(2));
        // Arrival times include connection scheduling, unlike the dispatch gate's clock.
        assert!(received[2].duration_since(received[1]) >= REQUEST_INTERVAL / 2);
    }

    #[test]
    fn flood_wait_parser_accepts_only_explicit_positive_delays() {
        for (message, seconds) in [
            ("Bad Request: too Many Requests: retry after 2", Some(2)),
            ("Too Many Requests: retry after 17", Some(17)),
            ("Bad Request: MESSAGE_ID_INVALID", None),
            ("Bad Request: retry after 2", None),
            ("Bad Request: too many requests: retry after -1", None),
            ("Bad Request: too many requests: retry after 0", None),
            ("Bad Request: too many requests: retry after 2 garbage", None),
            ("Bad Request: too many requests: retry after 99999999999999999999", None),
        ] {
            let error = SessionErrorKind::Telegram(TelegramErrorKind::BadRequest { message: message.into() });
            assert_eq!(telegram_retry_after(&error), seconds.map(Duration::from_secs), "{message}");
        }
    }

    #[tokio::test]
    async fn stream_upload_flood_wait_returns_error_without_replaying_consumed_bytes() {
        let fixture = Fixture::new().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut received = Vec::new();
            for index in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(count, 0);
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .map(|value| value.parse::<usize>().unwrap());
                        let complete = if let Some(length) = length {
                            request.len() >= end + 4 + length
                        } else {
                            headers.contains("transfer-encoding: chunked") && request.ends_with(b"0\r\n\r\n")
                        };
                        if complete {
                            break;
                        }
                    }
                }
                let path = String::from_utf8_lossy(&request).lines().next().unwrap().to_owned();
                received.push((path, Instant::now()));
                let (status, body) = if index == 0 {
                    (
                        "400 Bad Request",
                        r#"{"ok":false,"error_code":400,"description":"Bad Request: too Many Requests: retry after 2"}"#,
                    )
                } else {
                    ("200 OK", r#"{"ok":true,"result":true}"#)
                };
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len(),
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
            received
        });
        let api = APIServer::new(
            &format!("http://{address}/bot{{token}}/{{method_name}}"),
            &format!("http://{address}/files/{{path}}"),
            true,
            BareFilesPathWrapper,
        );
        let messenger = TelegramMessenger::new(
            Arc::new(Bot::with_client(
                "123:synthetic",
                Reqwest::default().with_api_server(Cow::Owned(api)),
            )),
            Arc::new(ErrorFormatter::new("synthetic")),
            Arc::new(TimeoutsConfig::default()),
            Arc::new(ProgressThrottle::new(fixture.connection())),
        );
        let temp_dir = TempDir::new().unwrap();
        let url = Url::parse("https://example.test/media").unwrap();
        let error = tokio::time::timeout(
            Duration::from_secs(3),
            messenger.upload_video(UploadVideoRequest {
                chat_id: 1,
                reply_to_message_id: None,
                media_for_upload: MediaForUpload {
                    path: temp_dir.path().join("media.mp4"),
                    thumb_stream: None,
                    temp_dir,
                    stream: MediaByteStream::new(stream::iter([Ok(Bytes::from_static(b"synthetic media"))])),
                    deadline: Instant::now() + Duration::from_secs(10),
                },
                name: "Synthetic media",
                width: None,
                height: None,
                duration: None,
                with_delete: false,
                webpage_url: &url,
                link_is_visible: false,
            }),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.to_string().contains("retry after 2"));
        assert!(messenger.limits.lock().await.cooldown_until > Instant::now());
        tokio::time::timeout(
            Duration::from_secs(5),
            messenger.delete_message(DeleteMessageRequest { chat_id: 1, message_id: 1 }),
        )
        .await
        .unwrap()
        .unwrap();
        let received = tokio::time::timeout(Duration::from_secs(1), server).await.unwrap().unwrap();
        assert!(received[0].0.contains("/sendVideo"));
        assert!(received[1].0.contains("/deleteMessage"));
        assert!(received[1].1.duration_since(received[0].1) >= Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn cooldown_wait_refreshes_queue_liveness_without_extending_upload_deadline() {
        let limits = Mutex::new(RequestLimits::default());
        limits.lock().await.cooldown_until = Instant::now() + Duration::from_secs(1_200);
        let outcome = downloader_client::outcome::ExecutionOutcome::default();
        let progress = outcome.subscribe_progress();
        let start = Instant::now();
        let deadline = start + Duration::from_secs(360);
        let result = outcome
            .scope(until_upload_deadline(
                async {
                    wait_for_request(&limits).await;
                    Ok(())
                },
                deadline,
            ))
            .await;
        assert!(result.is_err());
        assert_eq!(Instant::now(), deadline);
        assert!(progress.borrow().duration_since(start) >= Duration::from_secs(300));
        assert!(limits.lock().await.cooldown_until > deadline);
    }

    #[tokio::test(start_paused = true)]
    async fn final_error_cooldown_wait_stays_live_even_after_uncertain_execution() {
        let limits = Mutex::new(RequestLimits::default());
        let start = Instant::now();
        limits.lock().await.cooldown_until = start + Duration::from_secs(1_200);
        let outcome = downloader_client::outcome::ExecutionOutcome::default();
        outcome.mark_uncertain();
        let progress = outcome.subscribe_progress();
        outcome.scope(wait_for_request(&limits)).await;
        assert_eq!(Instant::now(), start + Duration::from_secs(1_200));
        assert!(Instant::now().duration_since(*progress.borrow()) <= WAIT_LIVENESS_INTERVAL);
        assert!(outcome.is_uncertain());
    }

    #[tokio::test(start_paused = true)]
    async fn queued_requests_recheck_an_extended_cooldown() {
        let limits = Arc::new(Mutex::new(RequestLimits::default()));
        limits.lock().await.cooldown_until = Instant::now() + Duration::from_secs(10);
        let waiter = {
            let limits = limits.clone();
            tokio::spawn(async move { wait_for_request(&limits).await })
        };
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(5)).await;
        limits.lock().await.cooldown_until = Instant::now() + Duration::from_secs(20);
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        tokio::time::advance(Duration::from_secs(15)).await;
        waiter.await.unwrap();
    }
}
