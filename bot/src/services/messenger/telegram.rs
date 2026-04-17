use crate::{
    config::TimeoutsConfig,
    entities::MediaForUpload,
    services::messenger::MessengerPort,
    utils::{media_link, sanitize_send_filename},
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
        InputMediaAudio, InputMediaVideo, InputTextMessageContent, LinkPreviewOptions, Message, ReplyParameters,
    },
    Bot,
};
use tracing::{error, warn};

use super::{
    AnswerInlineErrorRequest, AnswerInlineQueryRequest, DeleteMessageRequest, EditMediaByIdRequest, EditTarget, EditTextRequest,
    InlineQueryArticle, MessengerError, SendMediaByIdRequest, SendMediaGroupRequest, SendTextRequest, SentMessage, TextFormat,
    UploadAudioRequest, UploadVideoRequest,
};

pub struct TelegramMessenger {
    bot: Arc<Bot>,
    timeouts_cfg: Arc<TimeoutsConfig>,
}

impl TelegramMessenger {
    pub fn new(bot: Arc<Bot>, timeouts_cfg: Arc<TimeoutsConfig>) -> Self {
        Self { bot, timeouts_cfg }
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
                            .map(|message_id| ReplyParameters::new(message_id).allow_sending_without_reply(true)),
                    ),
            )
            .await?;
        Ok(SentMessage {
            message_id: message.message_id(),
        })
    }

    async fn edit_text(&self, request: EditTextRequest<'_>) -> Result<(), MessengerError> {
        let method = EditMessageText::new(request.text)
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
            .reply_parameters_option(reply_to_message_id.map(|val| ReplyParameters::new(val).allow_sending_without_reply(true)));

        let message = once(&self.bot, method, Some(self.timeouts_cfg.send_by_upload)).await?;
        drop(temp_dir);
        let message_id = message.message_id();
        let file_id = match message.video() {
            Some(video) => video.file_id.clone(),
            None => message.document().expect("Video upload returns video or document").file_id.clone(),
        };
        drop(message);

        if with_delete {
            tokio::spawn({
                let bot = self.bot.clone();
                async move {
                    if let Err(err) = bot.send(methods::DeleteMessage::new(chat_id, message_id)).await {
                        error!(%err, "Delete message error");
                    }
                }
            });
        }

        Ok(file_id.into())
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
            .reply_parameters_option(reply_to_message_id.map(|val| ReplyParameters::new(val).allow_sending_without_reply(true)));

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
            tokio::spawn({
                let bot = self.bot.clone();
                async move {
                    if let Err(err) = bot.send(methods::DeleteMessage::new(chat_id, message_id)).await {
                        error!(%err, "Delete message error");
                    }
                }
            });
        }

        Ok(file_id.into())
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

    async fn send_audio_group(&self, request: SendMediaGroupRequest) -> Result<(), MessengerError> {
        media_groups(
            &self.bot,
            request.chat_id,
            request
                .items
                .into_iter()
                .map(|item| {
                    InputMediaAudio::new(InputFile::id(item.remote_id))
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

async fn media_groups(
    bot: &Bot,
    chat_id: impl Into<ChatIdKind>,
    input_media_list: Vec<impl Into<InputMedia>>,
    reply_to_message_id: Option<i64>,
    request_timeout: Option<f32>,
) -> Result<Box<[Message]>, SessionErrorKind> {
    let chat_id = chat_id.into();
    let input_media_len = input_media_list.len();

    if input_media_len == 0 {
        return Ok(Box::new([]));
    }

    let cap = if input_media_len > 10 { 10 } else { input_media_len };

    let mut messages = Vec::with_capacity(input_media_len);
    let mut cur_media_group = Vec::with_capacity(cap);
    let mut cur_media_group_len = 0;

    for input_media in input_media_list {
        cur_media_group.push(input_media.into());
        cur_media_group_len += 1;

        if cur_media_group_len == 10 {
            let media_group = mem::take(&mut cur_media_group);
            let media_group_len = media_group.len();

            match with_retries(
                bot,
                SendMediaGroup::new(chat_id.clone(), media_group)
                    .disable_notification(true)
                    .reply_parameters_option(
                        reply_to_message_id
                            .map(|reply_to_message_id| ReplyParameters::new(reply_to_message_id).allow_sending_without_reply(true)),
                    ),
                3,
                request_timeout,
            )
            .await
            {
                Ok(new_messages) => messages.extend(new_messages),
                Err(_) => {
                    warn!("Skip {media_group_len} media count to send");
                }
            }

            cur_media_group_len = 0;
        }
    }

    if cur_media_group_len != 0 {
        messages.extend(
            with_retries(
                bot,
                SendMediaGroup::new(chat_id.clone(), cur_media_group)
                    .disable_notification(true)
                    .reply_parameters_option(
                        reply_to_message_id
                            .map(|reply_to_message_id| ReplyParameters::new(reply_to_message_id).allow_sending_without_reply(true)),
                    ),
                3,
                request_timeout,
            )
            .await?,
        );
    }

    Ok(messages.into())
}
