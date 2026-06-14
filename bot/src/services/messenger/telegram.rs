use crate::{
    config::TimeoutsConfig,
    entities::MediaForUpload,
    services::messenger::MessengerPort,
    utils::{media_link, sanitize_send_filename, ErrorFormatter},
};

use backoff::ExponentialBackoff;
use std::{
    mem,
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
use tracing::{error, warn};

use super::{
    AnswerInlineErrorRequest, AnswerInlineQueryRequest, DeleteMessageRequest, EditMediaByIdRequest, EditTarget, EditTextRequest,
    InlineQueryArticle, MessengerError, SendMediaByIdRequest, SendMediaGroupRequest, SendTextRequest, SentMessage, TextFormat,
    UploadAudioRequest, UploadPhotoRequest, UploadPhotoUrlRequest, UploadVideoRequest,
};

pub struct TelegramMessenger {
    bot: Arc<Bot>,
    error_formatter: Arc<ErrorFormatter>,
    timeouts_cfg: Arc<TimeoutsConfig>,
}

impl TelegramMessenger {
    pub fn new(bot: Arc<Bot>, error_formatter: Arc<ErrorFormatter>, timeouts_cfg: Arc<TimeoutsConfig>) -> Self {
        Self {
            bot,
            error_formatter,
            timeouts_cfg,
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
        let me = self.bot.send(GetMe {}).await?;
        Ok(me.username.expect("Bots always have a username").into())
    }

    async fn send_text(&self, request: SendTextRequest<'_>) -> Result<SentMessage, MessengerError> {
        let message = self
            .bot
            .send(
                SendMessage::new(request.chat_id, request.text)
                    .parse_mode_option(request.format.map(ParseMode::from))
                    .link_preview_options(LinkPreviewOptions::new().is_disabled(request.disable_link_preview))
                    .reply_parameters_option(
                        request
                            .reply_to_message_id
                            .map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)),
                    ),
            )
            .await?;
        Ok(SentMessage {
            message_id: message.message_id(),
        })
    }

    async fn edit_text(&self, request: EditTextRequest<'_>) -> Result<(), MessengerError> {
        let method = EditMessageText::new()
            .text(request.text)
            .parse_mode_option(request.format.map(ParseMode::from))
            .link_preview_options(LinkPreviewOptions::new().is_disabled(request.disable_link_preview));

        match request.target {
            EditTarget::ChatMessage { chat_id, message_id } => {
                self.bot.send(method.chat_id(chat_id).message_id(message_id)).await?;
            }
            EditTarget::InlineMessage { inline_message_id } => {
                let method = method.inline_message_id(inline_message_id);
                let method = if request.clear_inline_keyboard {
                    method.reply_markup(InlineKeyboardMarkup::new([[]]))
                } else {
                    method
                };
                self.bot.send(method).await?;
            }
        }
        Ok(())
    }

    async fn delete_message(&self, request: DeleteMessageRequest) -> Result<(), MessengerError> {
        self.bot.send(DeleteMessage::new(request.chat_id, request.message_id)).await?;
        Ok(())
    }

    async fn answer_inline_error(&self, request: AnswerInlineErrorRequest<'_>) -> Result<(), MessengerError> {
        let result = InlineQueryResultArticle::new(request.query_id, request.text, InputTextMessageContent::new(request.text));

        self.bot
            .send(AnswerInlineQuery::new(request.query_id, [result]).cache_time(0))
            .await?;
        Ok(())
    }

    async fn answer_inline_query(&self, request: AnswerInlineQueryRequest<'_>) -> Result<(), MessengerError> {
        let results: Vec<InlineQueryResult> = request.results.into_iter().map(InlineQueryResult::from).collect();

        self.bot
            .send(
                AnswerInlineQuery::new(request.query_id, results)
                    .cache_time(request.cache_time)
                    .is_personal(request.is_personal),
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
        let video = InputFile::stream_with_name(stream.into_inner(), &send_name);
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
            .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)));

        let message = once(&self.bot, method, Some(self.timeouts_cfg.send_by_upload)).await?;
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
        let audio = InputFile::stream_with_name(stream.into_inner(), &send_name);
        let thumbnail = thumb_stream.map(|stream| InputFile::stream_with_name(stream.into_inner(), "thumbnail.jpg"));
        let method = methods::SendAudio::new(chat_id, audio)
            .title_option(title)
            .duration_option(duration)
            .disable_notification(true)
            .performer_option(performer)
            .thumbnail_option(thumbnail)
            .caption_option(if link_is_visible { media_link(Some(webpage_url)) } else { None })
            .parse_mode(ParseMode::HTML)
            .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)));

        let message = once(&self.bot, method, Some(self.timeouts_cfg.send_by_upload)).await?;
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
            media_for_upload: MediaForUpload {
                path, temp_dir, stream, ..
            },
            name,
            with_delete,
            webpage_url,
            link_is_visible,
        } = request;
        let send_name = sanitize_send_filename(path.as_ref(), name);
        let photo = InputFile::stream_with_name(stream.into_inner(), &send_name);
        let method = methods::SendPhoto::new(chat_id, photo)
            .disable_notification(true)
            .caption_option(if link_is_visible { media_link(Some(webpage_url)) } else { None })
            .parse_mode(ParseMode::HTML)
            .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)));

        let message = once(&self.bot, method, Some(self.timeouts_cfg.send_by_upload)).await?;
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
            .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)));

        let message = once(&self.bot, method, Some(self.timeouts_cfg.send_by_upload)).await?;
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
            &self.bot,
            methods::SendVideo::new(request.chat_id, InputFile::id(request.remote_id))
                .reply_parameters_option(
                    request
                        .reply_to_message_id
                        .map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)),
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
            &self.bot,
            methods::SendAudio::new(request.chat_id, InputFile::id(request.remote_id))
                .reply_parameters_option(
                    request
                        .reply_to_message_id
                        .map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)),
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
            &self.bot,
            methods::SendPhoto::new(request.chat_id, InputFile::id(request.remote_id))
                .reply_parameters_option(
                    request
                        .reply_to_message_id
                        .map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)),
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
            &self.bot,
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
            &self.bot,
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
            &self.bot,
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
            &self.bot,
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
            &self.bot,
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
            &self.bot,
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
        let bot = self.bot.clone();
        let error_formatter = self.error_formatter.clone();
        tokio::spawn(async move {
            if let Err(err) = bot.send(methods::DeleteMessage::new(chat_id, message_id)).await {
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
async fn once<T>(bot: &Bot, method: T, request_timeout: Option<f32>) -> Result<T::Return, SessionErrorKind>
where
    T: TelegramMethod + Send + Sync,
    T::Method: Send + Sync,
{
    if let Some(request_timeout) = request_timeout {
        bot.send_with_timeout(method, request_timeout).await
    } else {
        bot.send(method).await
    }
}

#[allow(clippy::cast_sign_loss)]
async fn with_retries<T>(bot: &Bot, method: T, max_retries: u8, request_timeout: Option<f32>) -> Result<T::Return, SessionErrorKind>
where
    T: TelegramMethod + Clone + Send + Sync,
    T::Method: Send + Sync,
{
    let cur_retry_count = AtomicU8::new(0);

    backoff::future::retry(ExponentialBackoff::default(), || async {
        match once(bot, method.clone(), request_timeout).await {
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
    bot: &Bot,
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
                bot,
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
        if let Err(err) = send_media_group(bot, &chat_id, cur_media_group, reply_to_message_id, request_timeout, &mut messages).await {
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
    bot: &Bot,
    chat_id: &ChatIdKind,
    media_group: Vec<InputMedia>,
    reply_to_message_id: Option<i64>,
    request_timeout: Option<f32>,
    messages: &mut Vec<Message>,
) -> Result<(), SessionErrorKind> {
    let media_group_len = media_group.len();
    let res = with_retries(
        bot,
        SendMediaGroup::new(chat_id.clone(), media_group)
            .disable_notification(true)
            .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new(id).allow_sending_without_reply(true))),
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
