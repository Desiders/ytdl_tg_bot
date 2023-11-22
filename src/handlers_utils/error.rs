use telers::{
    enums::ParseMode,
    errors::SessionErrorKind,
    methods::{AnswerInlineQuery, EditMessageCaption, SendMessage},
    types::{InlineQueryResultArticle, InputTextMessageContent, Message},
    Bot,
};

pub async fn occured_in_message(
    bot: &Bot,
    chat_id: i64,
    reply_to_message_id: i64,
    text: &str,
    parse_mode: Option<ParseMode>,
) -> Result<Message, SessionErrorKind> {
    bot.send(
        SendMessage::new(chat_id, text)
            .reply_to_message_id(reply_to_message_id)
            .allow_sending_without_reply(true)
            .parse_mode_option(parse_mode),
    )
    .await
}

pub async fn occured_in_chosen_inline_result(
    bot: &Bot,
    text: &str,
    inline_message_id: &str,
    parse_mode: Option<ParseMode>,
) -> Result<(), SessionErrorKind> {
    bot.send(
        EditMessageCaption::new(text)
            .inline_message_id(inline_message_id)
            .parse_mode_option(parse_mode),
    )
    .await
    .map(|_| ())
}

pub async fn occured_in_inline_query_occured(bot: &Bot, query_id: &str, text: &str) -> Result<(), SessionErrorKind> {
    let result = InlineQueryResultArticle::new(query_id, text, InputTextMessageContent::new(text));
    let results = [result];

    bot.send(AnswerInlineQuery::new(query_id, results)).await.map(|_| ())
}
