use telers::{
    enums::ParseMode,
    methods::{AnswerInlineQuery, DeleteMessage, EditMessageText, GetMe, SendMessage},
    types::{
        InlineKeyboardButton, InlineKeyboardMarkup, InlineQueryResult, InlineQueryResultArticle, InputTextMessageContent,
        LinkPreviewOptions, ReplyParameters,
    },
    Bot,
};

use std::sync::Arc;

use super::{
    AnswerInlineErrorRequest, AnswerInlineQueryRequest, DeleteMessageRequest, EditTarget, EditTextRequest, InlineQueryArticle,
    MessengerError, MessengerPort, SendTextRequest, SentMessage, TextFormat,
};

pub struct TelegramMessenger {
    bot: Arc<Bot>,
}

impl TelegramMessenger {
    pub fn new(bot: Arc<Bot>) -> Self {
        Self { bot }
    }
}

impl From<telers::errors::SessionErrorKind> for MessengerError {
    fn from(value: telers::errors::SessionErrorKind) -> Self {
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
