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
    methods::{
        self, AnswerCallbackQuery, AnswerGuestQuery, AnswerInlineQuery, DeleteMessage, EditEphemeralMessageText, EditMessageText,
        GetChatAdministrators, GetMe, SendMediaGroup, SendMessage, TelegramMethod,
    },
    types::{
        ChatIdKind, ChatMember, EphemeralMessageParameters, InlineKeyboardButton, InlineKeyboardMarkup, InlineQueryResult,
        InlineQueryResultArticle, InputFile, InputMedia, InputMediaAudio, InputMediaPhoto, InputMediaVideo, InputTextMessageContent,
        LinkPreviewOptions, Message, ReplyParameters,
    },
    Bot,
};
use tokio::{
    sync::Mutex,
    time::{timeout, Instant},
};
use tracing::{error, warn};

const UPLOAD_TIMEOUT: Duration = Duration::from_secs(proto::MEDIA_EXECUTION_LIMIT_SECS);
const PROGRESS_EDIT_INTERVAL: Duration = Duration::from_secs(5);
const INLINE_PROGRESS_EDIT_INTERVAL: Duration = Duration::from_secs(15);
const WAIT_LIVENESS_INTERVAL: Duration = Duration::from_secs(30);
const REQUEST_INTERVAL: Duration = Duration::from_millis(100);
const ADMIN_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

use super::{
    AnswerCallbackRequest, AnswerGuestRequest, AnswerInlineErrorRequest, AnswerInlineQueryRequest, ButtonAction, DeleteMessageRequest,
    EditCaptionRequest, EditMediaByIdRequest, EditMenuRequest, EditTarget, EditTextRequest, InlineQueryArticle, Keyboard, MenuMessage,
    MessengerError, SendMediaByIdRequest, SendMediaGroupRequest, SendMenuRequest, SendTextRequest, SentMessage, TextFormat,
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

    async fn is_chat_administrator(&self, chat_id: i64, user_id: i64) -> Result<bool, MessengerError> {
        let administrators = timeout(
            ADMIN_CHECK_TIMEOUT,
            with_retries(
                self,
                GetChatAdministrators::new(chat_id),
                0,
                Some(ADMIN_CHECK_TIMEOUT.as_secs_f32()),
            ),
        )
        .await
        .map_err(|err| MessengerError::new(err.to_string()))??;
        Ok(administrators
            .iter()
            .any(|member| matches!(member, ChatMember::Creator(_) | ChatMember::Administrator(_)) && member.user().id == user_id))
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

    async fn edit_caption(&self, request: EditCaptionRequest<'_>) -> Result<(), MessengerError> {
        let method = methods::EditMessageCaption::new()
            .caption(request.text)
            .parse_mode_option(request.format.map(ParseMode::from));
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
        with_retries(self, method, 2, Some(self.timeouts_cfg.send_by_id)).await?;
        Ok(())
    }

    async fn delete_message(&self, request: DeleteMessageRequest) -> Result<(), MessengerError> {
        with_retries(self, DeleteMessage::new(request.chat_id, request.message_id), 0, None).await?;
        Ok(())
    }

    async fn send_menu(&self, request: SendMenuRequest<'_>) -> Result<(), MessengerError> {
        with_retries(
            self,
            SendMessage::new(request.chat_id, request.text)
                .parse_mode(ParseMode::HTML)
                .link_preview_options(LinkPreviewOptions::new().is_disabled(true))
                .ephemeral_message_parameters_option(
                    request
                        .ephemeral
                        .map(|target| EphemeralMessageParameters::new(target.receiver_user_id)),
                )
                .reply_parameters_option(
                    request
                        .ephemeral
                        .and_then(|target| target.reply_to_ephemeral_message_id)
                        .map(|id| ReplyParameters::new().ephemeral_message_id(id)),
                )
                .reply_markup(InlineKeyboardMarkup::from(request.keyboard)),
            0,
            None,
        )
        .await?;
        Ok(())
    }

    async fn edit_menu(&self, request: EditMenuRequest<'_>) -> Result<(), MessengerError> {
        let result = match request.message {
            MenuMessage::Chat { chat_id, message_id } => {
                let method = EditMessageText::new()
                    .chat_id(chat_id)
                    .message_id(message_id)
                    .text(request.text)
                    .parse_mode(ParseMode::HTML)
                    .link_preview_options(LinkPreviewOptions::new().is_disabled(true))
                    .reply_markup(InlineKeyboardMarkup::from(request.keyboard));
                with_retries(self, method, 0, None).await.map(|_| ())
            }
            MenuMessage::Ephemeral {
                chat_id,
                receiver_user_id,
                ephemeral_message_id,
            } => {
                let method = EditEphemeralMessageText::new(chat_id, receiver_user_id, ephemeral_message_id)
                    .text(request.text)
                    .parse_mode(ParseMode::HTML)
                    .link_preview_options(LinkPreviewOptions::new().is_disabled(true))
                    .reply_markup(InlineKeyboardMarkup::from(request.keyboard));
                with_retries(self, method, 0, None).await.map(|_| ())
            }
        };

        match result {
            Err(err) if !is_message_not_modified(&err) => Err(err.into()),
            _ => Ok(()),
        }
    }

    async fn answer_callback(&self, request: AnswerCallbackRequest<'_>) -> Result<(), MessengerError> {
        once(self, AnswerCallbackQuery::new(request.callback_id).text_option(request.text), None).await?;
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

    async fn answer_guest(&self, request: AnswerGuestRequest<'_>) -> Result<String, MessengerError> {
        let result = InlineQueryResultArticle::new(
            "guest",
            request.text,
            InputTextMessageContent::new(request.text).link_preview_options(LinkPreviewOptions::new().is_disabled(true)),
        );
        let sent = with_retries(self, AnswerGuestQuery::new(request.query_id, result), 0, None).await?;
        Ok(sent.inline_message_id.into())
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

    async fn send_video_by_id(&self, request: SendMediaByIdRequest<'_>) -> Result<SentMessage, MessengerError> {
        let message = with_retries(
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
        Ok(SentMessage {
            message_id: message.message_id(),
        })
    }

    async fn send_audio_by_id(&self, request: SendMediaByIdRequest<'_>) -> Result<SentMessage, MessengerError> {
        let message = with_retries(
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
        Ok(SentMessage {
            message_id: message.message_id(),
        })
    }

    async fn send_photo_by_id(&self, request: SendMediaByIdRequest<'_>) -> Result<SentMessage, MessengerError> {
        let message = with_retries(
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
        Ok(SentMessage {
            message_id: message.message_id(),
        })
    }

    async fn edit_video_by_id(&self, request: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        let method = methods::EditMessageMedia::new(
            InputMediaVideo::new(InputFile::id(request.remote_id))
                .caption_option(caption_with_link(
                    request.caption.map(ToOwned::to_owned),
                    request.link_is_visible,
                    request.webpage_url,
                ))
                .supports_streaming(true)
                .parse_mode(ParseMode::HTML),
        )
        .reply_markup(InlineKeyboardMarkup::new([[]]));
        let method = match request.target {
            EditTarget::ChatMessage { chat_id, message_id } => method.chat_id(chat_id).message_id(message_id),
            EditTarget::InlineMessage { inline_message_id } => method.inline_message_id(inline_message_id),
        };
        with_retries(self, method, 2, Some(self.timeouts_cfg.send_by_id)).await?;
        Ok(())
    }

    async fn edit_audio_by_id(&self, request: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        let method = methods::EditMessageMedia::new(
            InputMediaAudio::new(InputFile::id(request.remote_id))
                .caption_option(caption_with_link(
                    request.caption.map(ToOwned::to_owned),
                    request.link_is_visible,
                    request.webpage_url,
                ))
                .parse_mode(ParseMode::HTML),
        )
        .reply_markup(InlineKeyboardMarkup::new([[]]));
        let method = match request.target {
            EditTarget::ChatMessage { chat_id, message_id } => method.chat_id(chat_id).message_id(message_id),
            EditTarget::InlineMessage { inline_message_id } => method.inline_message_id(inline_message_id),
        };
        with_retries(self, method, 2, Some(self.timeouts_cfg.send_by_id)).await?;
        Ok(())
    }

    async fn edit_photo_by_id(&self, request: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        let method = methods::EditMessageMedia::new(
            InputMediaPhoto::new(InputFile::id(request.remote_id))
                .caption_option(caption_with_link(
                    request.caption.map(ToOwned::to_owned),
                    request.link_is_visible,
                    request.webpage_url,
                ))
                .parse_mode(ParseMode::HTML),
        )
        .reply_markup(InlineKeyboardMarkup::new([[]]));
        let method = match request.target {
            EditTarget::ChatMessage { chat_id, message_id } => method.chat_id(chat_id).message_id(message_id),
            EditTarget::InlineMessage { inline_message_id } => method.inline_message_id(inline_message_id),
        };
        with_retries(self, method, 2, Some(self.timeouts_cfg.send_by_id)).await?;
        Ok(())
    }

    async fn send_video_group(&self, request: SendMediaGroupRequest) -> Result<Option<SentMessage>, MessengerError> {
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
                .await
                .map(Some);
        }
        let messages = media_groups(
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
        Ok(messages.first().map(|message| SentMessage {
            message_id: message.message_id(),
        }))
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
    ) -> Result<Option<SentMessage>, MessengerError> {
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
                .await
                .map(Some);
        }
        let messages = media_groups(
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
        Ok(messages.first().map(|message| SentMessage {
            message_id: message.message_id(),
        }))
    }

    async fn send_photo_group(&self, request: SendMediaGroupRequest) -> Result<Option<SentMessage>, MessengerError> {
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
                .await
                .map(Some);
        }
        let messages = media_groups(
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
        Ok(messages.first().map(|message| SentMessage {
            message_id: message.message_id(),
        }))
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

impl From<&Keyboard> for InlineKeyboardMarkup {
    fn from(keyboard: &Keyboard) -> Self {
        InlineKeyboardMarkup::new(keyboard.rows.iter().map(|row| {
            row.iter()
                .map(|button| {
                    let telegram_button = InlineKeyboardButton::new(button.text.as_str());
                    match &button.action {
                        ButtonAction::Callback(data) => telegram_button.callback_data(data.as_str()),
                        ButtonAction::Url(url) => telegram_button.url(url.as_str()),
                    }
                })
                .collect::<Vec<_>>()
        }))
    }
}

// Pressing the button of the screen already shown edits to identical content, which the Bot API rejects.
//  https://github.com/tdlib/telegram-bot-api/blob/e3e9dd8e5b3d7ab8537cd5a10dc31d5ffa8f82d1/telegram-bot-api/Client.cpp#L110-L112
fn is_message_not_modified(error: &SessionErrorKind) -> bool {
    matches!(
        error,
        SessionErrorKind::Telegram(TelegramErrorKind::BadRequest { message })
            if message.to_ascii_lowercase().contains("message is not modified")
    )
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
            Err(err) if telegram_retry_after(&err).is_some() => {
                warn!(error = %err, "Waiting for Telegram cooldown");
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
    use crate::services::{
        messenger::{Button, EphemeralMenu, MediaGroupItem},
        queue::test_support::Fixture,
    };
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
        time::{pause, resume},
    };
    use url::Url;

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
    async fn media_playlists_return_the_first_delivered_message() {
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
        let sent = messenger
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
        assert_eq!(sent.unwrap().message_id, 1);
        assert!(server.await.unwrap().contains("/sendPhoto "));

        let server = reply_once(
            "200 OK",
            r#"{"ok":true,"result":[{"message_id":42,"date":1,"chat":{"id":1,"type":"private"}},{"message_id":43,"date":1,"chat":{"id":1,"type":"private"}}]}"#,
        )
        .await;
        let messenger = test_messenger(server.address, &fixture);
        let sent = messenger
            .send_photo_group(SendMediaGroupRequest {
                chat_id: 1,
                reply_to_message_id: None,
                items: vec![
                    MediaGroupItem {
                        remote_id: "synthetic-first-photo".into(),
                        webpage_url: None,
                    },
                    MediaGroupItem {
                        remote_id: "synthetic-second-photo".into(),
                        webpage_url: None,
                    },
                ],
                link_is_visible: false,
                caption: None,
            })
            .await
            .unwrap();
        assert_eq!(sent.unwrap().message_id, 42);
        assert!(server.request.await.unwrap().contains("/sendMediaGroup "));
        let sent = messenger
            .send_photo_group(SendMediaGroupRequest {
                chat_id: 1,
                reply_to_message_id: None,
                items: vec![],
                link_is_visible: false,
                caption: None,
            })
            .await
            .unwrap();
        assert!(sent.is_none());
    }

    #[tokio::test]
    async fn media_edits_support_chat_and_inline_targets() {
        let fixture = Fixture::new().await;
        let url = Url::parse("https://example.test/media?a=1&b=2").unwrap();
        let base_text = "<b>Synthetic artist — Synthetic song</b>";
        for kind in ["video", "audio", "photo"] {
            for is_inline in [false, true] {
                for link_is_visible in [false, true] {
                    let body = if is_inline {
                        r#"{"ok":true,"result":true}"#
                    } else {
                        r#"{"ok":true,"result":{"message_id":7,"date":1,"chat":{"id":321,"type":"private"}}}"#
                    };
                    let server = reply_once("200 OK", body).await;
                    let target = if is_inline {
                        EditTarget::InlineMessage {
                            inline_message_id: "synthetic-inline-message",
                        }
                    } else {
                        EditTarget::ChatMessage {
                            chat_id: 321,
                            message_id: 7,
                        }
                    };
                    let caption = (!is_inline).then_some(base_text);
                    let input = EditMediaByIdRequest {
                        target,
                        remote_id: "synthetic-file-id",
                        webpage_url: Some(&url),
                        link_is_visible,
                        caption,
                    };
                    let messenger = test_messenger(server.address, &fixture);
                    match kind {
                        "video" => messenger.edit_video_by_id(input).await,
                        "audio" => messenger.edit_audio_by_id(input).await,
                        _ => messenger.edit_photo_by_id(input).await,
                    }
                    .unwrap();
                    let request = server.request.await.unwrap();
                    assert!(request.contains("/editMessageMedia "));
                    assert!(request.contains(r#""inline_keyboard":[[]]"#));
                    if is_inline {
                        assert!(request.contains("name=\"inline_message_id\"\r\n\r\nsynthetic-inline-message"));
                        assert!(!request.contains("name=\"chat_id\""));
                    } else {
                        assert!(request.contains("name=\"chat_id\"\r\n\r\n321"));
                        assert!(request.contains("name=\"message_id\"\r\n\r\n7"));
                        assert!(!request.contains("name=\"inline_message_id\""));
                    }
                    let media = request.split_once("name=\"media\"\r\n\r\n").unwrap().1;
                    let media = media.split_once("\r\n--").unwrap().0;
                    let media: serde_json::Value = serde_json::from_str(media).unwrap();
                    assert_eq!(media["type"], kind);
                    assert_eq!(media["media"], "synthetic-file-id");
                    assert_eq!(media["parse_mode"], "HTML");
                    if kind == "video" {
                        assert_eq!(media["supports_streaming"], true);
                    }
                    let actual_caption = media["caption"].as_str().unwrap_or_default();
                    assert_eq!(actual_caption.contains(base_text), caption.is_some());
                    assert_eq!(actual_caption.contains(url.as_str()), link_is_visible);
                    if caption.is_some() && link_is_visible {
                        assert!(actual_caption.starts_with(&format!("{base_text}\n\n")));
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn post_text_and_caption_edits_keep_the_source_footer_and_content() {
        let fixture = Fixture::new().await;
        let text = concat!(
            "<blockquote>Synthetic post &amp; content</blockquote>\n\n",
            "<a href=\"https://example.test/post?a=1&amp;b=2\">Link</a>"
        );
        let server = reply_once(
            "200 OK",
            r#"{"ok":true,"result":{"message_id":1,"date":1,"chat":{"id":1,"type":"private"}}}"#,
        )
        .await;
        test_messenger(server.address, &fixture)
            .send_text(SendTextRequest {
                chat_id: 1,
                reply_to_message_id: Some(7),
                text,
                format: Some(TextFormat::Html),
                disable_link_preview: true,
            })
            .await
            .unwrap();
        let request = server.request.await.unwrap();
        assert!(request.contains("/sendMessage "));
        assert!(request.contains("name=\"text\""));
        assert!(request.contains(text));
        assert!(request.contains("HTML"));
        assert!(!request.contains("name=\"reply_markup\""));
        assert!(!request.contains(r#""url":"https://example.test/post"#));
        assert!(request.contains(r#""message_id":7"#));
        assert!(request.contains(r#""is_disabled":true"#));

        for as_caption in [true, false] {
            for (is_inline, clear_inline_keyboard) in [(false, false), (true, false), (true, true)] {
                let server = reply_once("200 OK", r#"{"ok":true,"result":true}"#).await;
                let target = if is_inline {
                    EditTarget::InlineMessage {
                        inline_message_id: "synthetic-inline-message",
                    }
                } else {
                    EditTarget::ChatMessage {
                        chat_id: 321,
                        message_id: 7,
                    }
                };
                let messenger = test_messenger(server.address, &fixture);
                if as_caption {
                    messenger
                        .edit_caption(EditCaptionRequest {
                            target,
                            text,
                            format: Some(TextFormat::Html),
                            clear_inline_keyboard,
                        })
                        .await
                } else {
                    messenger
                        .edit_text(EditTextRequest {
                            is_progress: false,
                            target,
                            text,
                            format: Some(TextFormat::Html),
                            disable_link_preview: true,
                            clear_inline_keyboard,
                        })
                        .await
                }
                .unwrap();
                let request = server.request.await.unwrap();
                let (method, field) = if as_caption {
                    ("editMessageCaption", "caption")
                } else {
                    ("editMessageText", "text")
                };
                assert!(request.contains(&format!("/{method} ")));
                assert!(request.contains(&format!("name=\"{field}\"")));
                assert!(request.contains(text));
                assert!(request.contains("HTML"));
                if clear_inline_keyboard {
                    assert!(request.contains(r#""inline_keyboard":[[]]"#));
                } else {
                    assert!(!request.contains("name=\"reply_markup\""));
                }
                assert!(!request.contains(r#""url":"https://example.test/post"#));
                assert!(!request.contains("name=\"media\""));
                if is_inline {
                    assert!(request.contains("synthetic-inline-message"));
                } else {
                    assert!(request.contains("name=\"chat_id\"\r\n\r\n321"));
                    assert!(request.contains("name=\"message_id\"\r\n\r\n7"));
                }
                if !as_caption {
                    assert!(request.contains(r#""is_disabled":true"#));
                }
            }
        }
    }

    #[tokio::test]
    async fn administrator_checks_accept_owners_and_administrators_and_reject_other_users() {
        let fixture = Fixture::new().await;
        let body = r#"{"ok":true,"result":[
            {"status":"creator","user":{"id":1,"is_bot":false,"first_name":"Synthetic owner"},"is_anonymous":false},
            {"status":"administrator","user":{"id":2,"is_bot":false,"first_name":"Synthetic administrator"},
             "can_be_edited":false,"is_anonymous":false,"can_manage_chat":true,"can_delete_messages":false,
             "can_manage_video_chats":false,"can_restrict_members":false,"can_promote_members":false,
             "can_change_info":false,"can_invite_users":false,"can_post_stories":false,"can_edit_stories":false,
             "can_delete_stories":false,"can_send_welcome_messages":false}
        ]}"#;
        for (user_id, expected) in [(1, true), (2, true), (3, false)] {
            let server = reply_once("200 OK", body).await;
            let actual = test_messenger(server.address, &fixture)
                .is_chat_administrator(-100, user_id)
                .await
                .unwrap();
            assert_eq!(actual, expected);
            let request = server.request.await.unwrap();
            assert!(request.contains("/getChatAdministrators "));
            assert!(request.contains("name=\"chat_id\"\r\n\r\n-100"));
        }

        let server = reply_once(
            "400 Bad Request",
            r#"{"ok":false,"error_code":400,"description":"Bad Request: chat not found"}"#,
        )
        .await;
        assert!(test_messenger(server.address, &fixture)
            .is_chat_administrator(-100, 1)
            .await
            .is_err());
        server.request.await.unwrap();
    }

    #[tokio::test]
    async fn administrator_checks_have_a_deadline_while_telegram_is_in_cooldown() {
        let fixture = Fixture::new().await;
        let messenger = test_messenger("127.0.0.1:1".parse().unwrap(), &fixture);
        pause();
        messenger.limits.lock().await.cooldown_until = Instant::now() + Duration::from_secs(60);
        let started = Instant::now();
        assert!(messenger.is_chat_administrator(-100, 1).await.is_err());
        assert!(started.elapsed() >= ADMIN_CHECK_TIMEOUT);
        assert!(started.elapsed() < Duration::from_secs(6));
        resume();
    }

    #[tokio::test]
    async fn menus_send_privately_or_to_the_ephemeral_receiver() {
        let fixture = Fixture::new().await;
        for ephemeral in [
            None,
            Some(EphemeralMenu {
                receiver_user_id: 1,
                reply_to_ephemeral_message_id: None,
            }),
            Some(EphemeralMenu {
                receiver_user_id: 1,
                reply_to_ephemeral_message_id: Some(7),
            }),
        ] {
            let (chat_id, body) = if ephemeral.is_some() {
                (
                    -100,
                    r#"{"ok":true,"result":{"message_id":0,"date":1,"chat":{"id":-100,"type":"group","title":"Synthetic group"},"receiver_user":{"id":1,"is_bot":false,"first_name":"Synthetic user"},"ephemeral_message_id":8,"text":"Synthetic menu"}}"#,
                )
            } else {
                (
                    1,
                    r#"{"ok":true,"result":{"message_id":8,"date":1,"chat":{"id":1,"type":"private"},"text":"Synthetic menu"}}"#,
                )
            };
            let server = reply_once("200 OK", body).await;
            test_messenger(server.address, &fixture)
                .send_menu(SendMenuRequest {
                    chat_id,
                    ephemeral,
                    text: "Synthetic menu",
                    keyboard: &Keyboard { rows: Vec::new() },
                })
                .await
                .unwrap();
            let request = server.request.await.unwrap();
            assert!(request.contains("/sendMessage "));
            assert!(request.contains(&format!("name=\"chat_id\"\r\n\r\n{chat_id}")));
            assert_eq!(request.contains("name=\"ephemeral_message_parameters\""), ephemeral.is_some());
            assert_eq!(request.contains(r#""receiver_user_id":1"#), ephemeral.is_some());
            assert_eq!(
                request.contains(r#""ephemeral_message_id":7"#),
                ephemeral.and_then(|target| target.reply_to_ephemeral_message_id).is_some()
            );
            assert!(!request.contains(r#""message_id":"#));
            assert!(!request.contains("allow_sending_without_reply"));
        }
    }

    #[tokio::test]
    async fn ephemeral_menu_rejections_do_not_send_a_public_menu() {
        let fixture = Fixture::new().await;
        let server = reply_once(
            "400 Bad Request",
            r#"{"ok":false,"error_code":400,"description":"Bad Request: ephemeral messages are unavailable"}"#,
        )
        .await;
        let error = test_messenger(server.address, &fixture)
            .send_menu(SendMenuRequest {
                chat_id: -100,
                ephemeral: Some(EphemeralMenu {
                    receiver_user_id: 1,
                    reply_to_ephemeral_message_id: None,
                }),
                text: "Synthetic menu",
                keyboard: &Keyboard { rows: Vec::new() },
            })
            .await
            .unwrap_err();
        let request = server.request.await.unwrap();
        assert!(request.contains("/sendMessage "));
        assert!(request.contains(r#""receiver_user_id":1"#));
        assert!(error.to_string().contains("ephemeral messages are unavailable"), "{error}");
    }

    #[tokio::test]
    async fn menu_edits_keep_the_keyboard_and_accept_unchanged_messages() {
        let fixture = Fixture::new().await;
        let keyboard = Keyboard {
            rows: vec![
                vec![Button {
                    text: "Synthetic screen".into(),
                    action: ButtonAction::Callback("open:synthetic".into()),
                }],
                vec![Button {
                    text: "Synthetic link".into(),
                    action: ButtonAction::Url("https://example.com/".into()),
                }],
            ],
        };

        for message in [
            MenuMessage::Chat { chat_id: 1, message_id: 2 },
            MenuMessage::Ephemeral {
                chat_id: -100,
                receiver_user_id: 1,
                ephemeral_message_id: 2,
            },
        ] {
            for (status, body) in [
                ("200 OK", r#"{"ok":true,"result":true}"#),
                (
                    "400 Bad Request",
                    r#"{"ok":false,"error_code":400,"description":"Bad Request: message is not modified: specified new message content and reply markup are exactly the same as a current content and reply markup of the message"}"#,
                ),
            ] {
                let server = reply_once(status, body).await;
                test_messenger(server.address, &fixture)
                    .edit_menu(EditMenuRequest {
                        message,
                        text: "Synthetic menu",
                        keyboard: &keyboard,
                    })
                    .await
                    .unwrap();

                let request = server.request.await.unwrap();
                match message {
                    MenuMessage::Chat { chat_id, message_id } => {
                        assert!(request.contains("/editMessageText "));
                        assert!(request.contains(&format!("name=\"chat_id\"\r\n\r\n{chat_id}")));
                        assert!(request.contains(&format!("name=\"message_id\"\r\n\r\n{message_id}")));
                        assert!(!request.contains("name=\"receiver_user_id\""));
                        assert!(!request.contains("name=\"ephemeral_message_id\""));
                    }
                    MenuMessage::Ephemeral {
                        chat_id,
                        receiver_user_id,
                        ephemeral_message_id,
                    } => {
                        assert!(request.contains("/editEphemeralMessageText "));
                        assert!(request.contains(&format!("name=\"chat_id\"\r\n\r\n{chat_id}")));
                        assert!(request.contains(&format!("name=\"receiver_user_id\"\r\n\r\n{receiver_user_id}")));
                        assert!(request.contains(&format!("name=\"ephemeral_message_id\"\r\n\r\n{ephemeral_message_id}")));
                        assert!(!request.contains("name=\"message_id\""));
                    }
                }
                assert!(request.contains(r#""callback_data":"open:synthetic""#), "{request}");
                assert!(request.contains(r#""url":"https://example.com/""#), "{request}");
                assert!(request.contains("name=\"parse_mode\"\r\n\r\nHTML"));
                assert!(request.contains(r#""is_disabled":true"#));
            }
        }
    }

    #[tokio::test]
    async fn edit_menu_reports_other_rejections() {
        let fixture = Fixture::new().await;
        for message in [
            MenuMessage::Chat { chat_id: 1, message_id: 2 },
            MenuMessage::Ephemeral {
                chat_id: -100,
                receiver_user_id: 1,
                ephemeral_message_id: 2,
            },
        ] {
            let server = reply_once(
                "400 Bad Request",
                r#"{"ok":false,"error_code":400,"description":"Bad Request: message to edit not found"}"#,
            )
            .await;

            let error = test_messenger(server.address, &fixture)
                .edit_menu(EditMenuRequest {
                    message,
                    text: "Synthetic menu",
                    keyboard: &Keyboard { rows: Vec::new() },
                })
                .await
                .unwrap_err();

            server.request.await.unwrap();
            assert!(error.to_string().contains("message to edit not found"), "{error}");
        }
    }

    #[tokio::test]
    async fn answer_callback_sends_the_notice() {
        let fixture = Fixture::new().await;
        let server = reply_once("200 OK", r#"{"ok":true,"result":true}"#).await;

        test_messenger(server.address, &fixture)
            .answer_callback(AnswerCallbackRequest {
                callback_id: "synthetic-callback",
                text: Some("Synthetic notice"),
            })
            .await
            .unwrap();

        let request = server.request.await.unwrap();
        assert!(request.contains("/answerCallbackQuery "));
        assert!(request.contains("synthetic-callback"), "{request}");
        assert!(request.contains("Synthetic notice"), "{request}");
    }

    struct FakeServer {
        address: SocketAddr,
        request: tokio::task::JoinHandle<String>,
    }

    async fn reply_once(status: &'static str, body: &'static str) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        let request = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        FakeServer { address, request }
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
