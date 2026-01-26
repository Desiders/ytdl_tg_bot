use telers::{
    enums::ParseMode,
    errors::SessionErrorKind,
    methods::{AnswerInlineQuery, DeleteMessage, EditMessageText, SendMessage},
    types::{InlineKeyboardMarkup, InlineQueryResultArticle, InputTextMessageContent, LinkPreviewOptions, Message, ReplyParameters},
    utils::text::{html_expandable_blockquote, html_quote},
    Bot,
};

pub async fn new(bot: &Bot, chat_id: i64, reply_to_message_id: i64) -> Result<Message, SessionErrorKind> {
    bot.send(
        SendMessage::new(chat_id, "🔍 Preparing download…")
            .link_preview_options(LinkPreviewOptions::new().is_disabled(true))
            .reply_parameters(ReplyParameters::new(reply_to_message_id).allow_sending_without_reply(true)),
    )
    .await
}

pub async fn is_downloading(bot: &Bot, chat_id: i64, message_id: i64) -> Result<(), SessionErrorKind> {
    bot.send(
        EditMessageText::new("📥 Downloading...")
            .chat_id(chat_id)
            .message_id(message_id)
            .link_preview_options(LinkPreviewOptions::new().is_disabled(true)),
    )
    .await?;
    Ok(())
}

pub async fn is_sending_with_errors_or_all_errors(
    bot: &Bot,
    chat_id: i64,
    message_id: i64,
    errs: &[String],
    media_to_send_count: usize,
) -> Result<(), SessionErrorKind> {
    let text = match (errs.len(), media_to_send_count) {
        (errs_count, media_to_send_count) if errs_count > 0 && media_to_send_count > 0 => {
            let mut text = "📨 Sending...\n\n🧨 Error while downloading some media :(\n\n".to_owned();
            for (index, err) in errs.into_iter().enumerate() {
                text.push_str(&html_expandable_blockquote(format!("{}. {}", index + 1, html_quote(err))));
                text.push('\n');
            }
            text
        }
        (errs_count, media_to_send_count) if errs_count > 0 && media_to_send_count == 0 => {
            let mut text = "🧨 Error while downloading :(\n\n".to_owned();
            for (index, err) in errs.into_iter().enumerate() {
                text.push_str(&html_expandable_blockquote(format!("{}. {}", index + 1, html_quote(err))));
                text.push('\n');
            }
            text
        }
        (_, _) => return Ok(()),
    };

    bot.send(
        EditMessageText::new(text)
            .chat_id(chat_id)
            .message_id(message_id)
            .parse_mode(ParseMode::HTML)
            .link_preview_options(LinkPreviewOptions::new().is_disabled(true)),
    )
    .await?;
    Ok(())
}

pub async fn is_downloading_with_progress(
    bot: &Bot,
    chat_id: i64,
    message_id: i64,
    progress: String,
    current_media_index: usize,
    playlist_len: usize,
) -> Result<(), SessionErrorKind> {
    let text = if playlist_len > 1 {
        format!(
            "📥 Downloading playlist... {current_media_index}/{playlist_len}\n\n\
            Media download progress: {progress}"
        )
    } else {
        format!(
            "📥 Downloading...\n\n\
            Media download progress: {progress}"
        )
    };
    bot.send(EditMessageText::new(text).chat_id(chat_id).message_id(message_id)).await?;
    Ok(())
}

pub async fn is_error(bot: &Bot, chat_id: i64, message_id: i64, text: &str, parse_mode: Option<ParseMode>) -> Result<(), SessionErrorKind> {
    bot.send(
        EditMessageText::new(text)
            .chat_id(chat_id)
            .message_id(message_id)
            .parse_mode_option(parse_mode)
            .link_preview_options(LinkPreviewOptions::new().is_disabled(true)),
    )
    .await?;
    Ok(())
}

pub async fn is_error_in_chosen_inline(
    bot: &Bot,
    inline_message_id: &str,
    text: &str,
    parse_mode: Option<ParseMode>,
) -> Result<(), SessionErrorKind> {
    bot.send(
        EditMessageText::new(text)
            .inline_message_id(inline_message_id)
            .reply_markup(InlineKeyboardMarkup::new([[]]))
            .parse_mode_option(parse_mode)
            .link_preview_options(LinkPreviewOptions::new().is_disabled(true)),
    )
    .await?;
    Ok(())
}

pub async fn is_error_in_inline_query(bot: &Bot, query_id: &str, text: &str) -> Result<(), SessionErrorKind> {
    let result = InlineQueryResultArticle::new(query_id, text, InputTextMessageContent::new(text));
    let results = [result];

    bot.send(AnswerInlineQuery::new(query_id, results).cache_time(0)).await?;
    Ok(())
}

pub async fn delete(bot: &Bot, chat_id: i64, message_id: i64) -> Result<(), SessionErrorKind> {
    bot.send(DeleteMessage::new(chat_id, message_id)).await?;
    Ok(())
}
