use std::fmt::Write as _;
use telers::{
    enums::ParseMode,
    errors::SessionErrorKind,
    methods::{AnswerInlineQuery, DeleteMessage, EditMessageText, SendMessage},
    types::{InlineKeyboardMarkup, InlineQueryResultArticle, InputTextMessageContent, LinkPreviewOptions, Message, ReplyParameters},
    utils::text::html_expandable_blockquote,
    Bot,
};

pub async fn new(
    bot: &Bot,
    text: &str,
    chat_id: i64,
    reply_to_message_id: Option<i64>,
    parse_mode: Option<ParseMode>,
) -> Result<Message, SessionErrorKind> {
    bot.send(
        SendMessage::new(chat_id, text)
            .link_preview_options(LinkPreviewOptions::new().is_disabled(true))
            .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)))
            .parse_mode_option(parse_mode),
    )
    .await
}

pub async fn is_sending(bot: &Bot, chat_id: i64, message_id: i64) -> Result<(), SessionErrorKind> {
    bot.send(
        EditMessageText::new("📨 Sending...")
            .chat_id(chat_id)
            .message_id(message_id)
            .parse_mode(ParseMode::HTML)
            .link_preview_options(LinkPreviewOptions::new().is_disabled(true)),
    )
    .await?;
    Ok(())
}

pub async fn is_errors_if_exist(
    bot: &Bot,
    chat_id: i64,
    message_id: i64,
    media_errs: &[Vec<String>],
    media_to_send_count: usize,
) -> Result<(), SessionErrorKind> {
    let mut errs_text = String::new();
    for (failed_media_index, format_errs) in media_errs.iter().enumerate() {
        let mut format_errs_text = String::new();
        for (format_err_index, format_err) in format_errs.iter().enumerate() {
            let _ = writeln!(format_errs_text, "{}. {}", format_err_index + 1, format_err);
        }
        let _ = writeln!(errs_text, "{} media ({} download retries):", failed_media_index + 1, format_errs.len());
        errs_text.push_str(&html_expandable_blockquote(&format_errs_text));
    }

    let text = match (media_errs.len(), media_to_send_count) {
        (errs_count, media_to_send_count) if errs_count > 0 && media_to_send_count > 0 => {
            format!(
                "🧨 Error while downloading some media :(\n\n\
                {}",
                html_expandable_blockquote(&errs_text),
            )
        }
        (errs_count, media_to_send_count) if errs_count > 0 && media_to_send_count == 0 => {
            format!(
                "🧨 Error while downloading :(\n\n\
                {}",
                html_expandable_blockquote(&errs_text),
            )
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

pub async fn is_sending_in_chosen_inline(bot: &Bot, inline_message_id: &str) -> Result<(), SessionErrorKind> {
    bot.send(
        EditMessageText::new("📨 Sending...")
            .inline_message_id(inline_message_id)
            .reply_markup(InlineKeyboardMarkup::new([[]]))
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

pub async fn is_downloading_with_progress_in_chosen_inline(
    bot: &Bot,
    inline_message_id: &str,
    progress: String,
) -> Result<(), SessionErrorKind> {
    let text = format!(
        "📥 Downloading...\n\n\
        Media download progress: {progress}"
    );
    bot.send(
        EditMessageText::new(text)
            .inline_message_id(inline_message_id)
            .reply_markup(InlineKeyboardMarkup::new([[]]))
            .link_preview_options(LinkPreviewOptions::new().is_disabled(true)),
    )
    .await?;
    Ok(())
}

pub async fn is_error_in_progress(
    bot: &Bot,
    chat_id: i64,
    message_id: i64,
    text: &str,
    parse_mode: Option<ParseMode>,
) -> Result<(), SessionErrorKind> {
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

pub async fn is_errors_in_chosen_inline(
    bot: &Bot,
    inline_message_id: &str,
    format_errs: &[String],
    parse_mode: Option<ParseMode>,
) -> Result<(), SessionErrorKind> {
    let mut errs_text = String::new();
    for (index, err) in format_errs.iter().enumerate() {
        let _ = writeln!(errs_text, "{}. {}", index + 1, err);
    }
    let _ = writeln!(errs_text, "{} download retries:", format_errs.len());
    errs_text.push_str(&html_expandable_blockquote(&errs_text));

    let text = format!(
        "🧨 Error while downloading :(\n\n\
        {}",
        html_expandable_blockquote(&errs_text),
    );
    is_error_in_chosen_inline(bot, inline_message_id, &text, parse_mode).await
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
