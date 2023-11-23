use telers::{
    enums::ParseMode,
    errors::SessionErrorKind,
    methods::{AnswerInlineQuery, EditMessageCaption, SendMessage},
    types::{InlineKeyboardMarkup, InlineQueryResultArticle, InputTextMessageContent, Message},
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
            .parse_mode_option(parse_mode)
            .reply_markup(InlineKeyboardMarkup::new([[]])),
    )
    .await
    .map(|_| ())
}

pub async fn occured_in_inline_query_occured(bot: &Bot, query_id: &str, text: &str) -> Result<(), SessionErrorKind> {
    let result = InlineQueryResultArticle::new(query_id, text, InputTextMessageContent::new(text));
    let results = [result];

    bot.send(AnswerInlineQuery::new(query_id, results)).await.map(|_| ())
}

pub async fn download_videos_in_message(
    bot: &Bot,
    count: usize,
    chat_id: i64,
    reply_to_message_id: i64,
    parse_mode: Option<ParseMode>,
) -> Result<(), SessionErrorKind> {
    let text = if count == 1 {
        "Sorry, an error occurred while downloading the video. Try again later.".to_owned()
    } else {
        format!("Sorry, an error occurred while downloading {count} videos from the playlist. Try again later.")
    };

    occured_in_message(bot, chat_id, reply_to_message_id, &text, parse_mode)
        .await
        .map(|_| ())
}
