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
    io, mem,
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
const REQUEST_INTERVAL: Duration = Duration::from_millis(100);

use super::{
    AnswerInlineErrorRequest, AnswerInlineQueryRequest, DeleteMessageRequest, EditMediaByIdRequest, EditTarget, EditTextRequest,
    InlineQueryArticle, MessengerError, SendMediaByIdRequest, SendMediaGroupRequest, SendTextRequest, SentMessage, TextFormat,
    UploadAudioRequest, UploadPhotoRequest, UploadPhotoUrlRequest, UploadVideoRequest,
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
        Self::new(value.to_string())
    }
}

impl MessengerPort for TelegramMessenger {
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
            if !matches!(&result, Err(SessionErrorKind::Telegram(TelegramErrorKind::RetryAfter { .. }))) {
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
        tokio::time::sleep_until(ready).await;
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
    if let Err(SessionErrorKind::Telegram(TelegramErrorKind::RetryAfter { retry_after, .. })) = &result {
        let delay = Duration::from_secs(u64::try_from(*retry_after).unwrap_or_default());
        let mut limits = messenger.limits.lock().await;
        limits.cooldown_until = limits.cooldown_until.max(Instant::now() + delay);
    }
    if result.is_ok() {
        record_execution_progress();
    }
    result
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
        .map_err(|_| MessengerError::new("Media execution exceeded the per-media time limit"))?
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
            Err(err) => Err(match err {
                SessionErrorKind::Telegram(TelegramErrorKind::RetryAfter { retry_after, .. }) => {
                    warn!("Sleeping for {retry_after:?} seconds");
                    backoff::Error::retry_after(err, Duration::from_secs(retry_after.try_into().unwrap()))
                }
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

/// Telegram media groups are limited to 10 items; this splits a longer list
/// into 10-sized batches and tolerates per-batch failures.
async fn media_groups(
    messenger: &TelegramMessenger,
    chat_id: impl Into<ChatIdKind>,
    input_media_list: Vec<impl Into<InputMedia>>,
    reply_to_message_id: Option<i64>,
    request_timeout: Option<f32>,
) -> Result<Box<[Message]>, SessionErrorKind> {
    const MAX_MEDIA_GROUP: usize = 10;

    let chat_id = chat_id.into();
    let input_media_len = input_media_list.len();

    if input_media_len == 0 {
        return Ok(Box::new([]));
    }

    let mut messages = Vec::with_capacity(input_media_len);
    let mut cur_media_group = Vec::with_capacity(input_media_len.min(MAX_MEDIA_GROUP));
    let mut last_error = None;

    for input_media in input_media_list {
        cur_media_group.push(input_media.into());

        if cur_media_group.len() == MAX_MEDIA_GROUP {
            if let Err(err) = send_media_group(
                messenger,
                &chat_id,
                mem::take(&mut cur_media_group),
                reply_to_message_id,
                request_timeout,
                &mut messages,
            )
            .await
            {
                last_error = Some(err);
            }
        }
    }

    if !cur_media_group.is_empty() {
        if let Err(err) = send_media_group(
            messenger,
            &chat_id,
            cur_media_group,
            reply_to_message_id,
            request_timeout,
            &mut messages,
        )
        .await
        {
            last_error = Some(err);
        }
    }

    // Tolerate partial failures (some batches sent), but if nothing went through, surface the
    // error so the caller reports it instead of silently deleting the progress message.
    if messages.is_empty() {
        if let Some(err) = last_error {
            return Err(err);
        }
    }

    Ok(messages.into())
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
    use crate::services::queue::test_support::Fixture;
    use std::borrow::Cow;
    use telers::client::{
        telegram::{APIServer, BareFilesPathWrapper},
        Reqwest,
    };
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
    };

    #[tokio::test]
    async fn telegram_cooldown_is_shared_and_final_error_edits_are_not_dropped() {
        let fixture = Fixture::new().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut received = Vec::new();
            for index in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
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
                            break;
                        }
                    }
                }
                received.push(Instant::now());
                let (status, body) = if index == 0 {
                    (
                        "429 Too Many Requests",
                        r#"{"ok":false,"error_code":429,"description":"Too Many Requests","parameters":{"retry_after":2}}"#,
                    )
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

    #[tokio::test(start_paused = true)]
    async fn cooldown_blocks_requests_and_drops_progress_without_refreshing_liveness() {
        let limits = Mutex::new(RequestLimits::default());
        limits.lock().await.cooldown_until = Instant::now() + Duration::from_secs(20);
        let outcome = downloader_client::outcome::ExecutionOutcome::default();
        let progress = outcome.subscribe_progress();
        let last_progress = *progress.borrow();
        assert!(limits.lock().await.cooldown_until > Instant::now());
        assert!(tokio::time::timeout(Duration::from_secs(19), wait_for_request(&limits))
            .await
            .is_err());
        tokio::time::advance(Duration::from_secs(1)).await;
        wait_for_request(&limits).await;
        assert_eq!(*progress.borrow(), last_progress);
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
