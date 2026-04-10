use std::fmt::Write as _;
use telers::utils::text::html_expandable_blockquote;

use crate::services::messenger::{
    AnswerInlineErrorRequest, DeleteMessageRequest, EditTarget, EditTextRequest, MessengerError, MessengerPort, SendTextRequest,
    SentMessage, TextFormat,
};

pub async fn new(
    messenger: &(impl MessengerPort + ?Sized),
    text: &str,
    chat_id: i64,
    reply_to_message_id: Option<i64>,
    format: Option<TextFormat>,
) -> Result<SentMessage, MessengerError> {
    messenger
        .send_text(SendTextRequest {
            chat_id,
            text,
            reply_to_message_id,
            format,
            disable_link_preview: true,
        })
        .await
}

pub async fn is_sending(messenger: &(impl MessengerPort + ?Sized), chat_id: i64, message_id: i64) -> Result<(), MessengerError> {
    messenger
        .edit_text(EditTextRequest {
            target: EditTarget::ChatMessage { chat_id, message_id },
            text: "📨 Sending...",
            format: Some(TextFormat::Html),
            disable_link_preview: true,
            clear_inline_keyboard: false,
        })
        .await
}

pub async fn is_errors_if_exist(
    messenger: &(impl MessengerPort + ?Sized),
    chat_id: i64,
    message_id: i64,
    media_errs: &[Vec<String>],
    media_to_send_count: usize,
) -> Result<(), MessengerError> {
    let mut errs_text = String::new();
    for (failed_media_index, format_errs) in media_errs.iter().enumerate() {
        let mut format_errs_text = String::new();
        for (format_err_index, format_err) in format_errs.iter().enumerate() {
            let _ = writeln!(format_errs_text, "{}. {}", format_err_index + 1, format_err);
        }
        let _ = writeln!(
            errs_text,
            "{} media ({} download retries):",
            failed_media_index + 1,
            format_errs.len()
        );
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

    messenger
        .edit_text(EditTextRequest {
            target: EditTarget::ChatMessage { chat_id, message_id },
            text: &text,
            format: Some(TextFormat::Html),
            disable_link_preview: true,
            clear_inline_keyboard: false,
        })
        .await
}

pub async fn is_sending_in_chosen_inline(messenger: &(impl MessengerPort + ?Sized), inline_message_id: &str) -> Result<(), MessengerError> {
    messenger
        .edit_text(EditTextRequest {
            target: EditTarget::InlineMessage { inline_message_id },
            text: "📨 Sending...",
            format: None,
            disable_link_preview: true,
            clear_inline_keyboard: true,
        })
        .await
}

pub async fn is_downloading_with_progress(
    messenger: &(impl MessengerPort + ?Sized),
    chat_id: i64,
    message_id: i64,
    progress: String,
    current_media_index: usize,
    playlist_len: usize,
) -> Result<(), MessengerError> {
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
    messenger
        .edit_text(EditTextRequest {
            target: EditTarget::ChatMessage { chat_id, message_id },
            text: &text,
            format: None,
            disable_link_preview: false,
            clear_inline_keyboard: false,
        })
        .await
}

pub async fn is_downloading_with_progress_in_chosen_inline(
    messenger: &(impl MessengerPort + ?Sized),
    inline_message_id: &str,
    progress: String,
) -> Result<(), MessengerError> {
    let text = format!(
        "📥 Downloading...\n\n\
        Media download progress: {progress}"
    );
    messenger
        .edit_text(EditTextRequest {
            target: EditTarget::InlineMessage { inline_message_id },
            text: &text,
            format: None,
            disable_link_preview: true,
            clear_inline_keyboard: true,
        })
        .await
}

pub async fn is_error_in_progress(
    messenger: &(impl MessengerPort + ?Sized),
    chat_id: i64,
    message_id: i64,
    text: &str,
    format: Option<TextFormat>,
) -> Result<(), MessengerError> {
    messenger
        .edit_text(EditTextRequest {
            target: EditTarget::ChatMessage { chat_id, message_id },
            text,
            format,
            disable_link_preview: true,
            clear_inline_keyboard: false,
        })
        .await
}

pub async fn is_error_in_chosen_inline(
    messenger: &(impl MessengerPort + ?Sized),
    inline_message_id: &str,
    text: &str,
    format: Option<TextFormat>,
) -> Result<(), MessengerError> {
    messenger
        .edit_text(EditTextRequest {
            target: EditTarget::InlineMessage { inline_message_id },
            text,
            format,
            disable_link_preview: true,
            clear_inline_keyboard: true,
        })
        .await
}

pub async fn is_errors_in_chosen_inline(
    messenger: &(impl MessengerPort + ?Sized),
    inline_message_id: &str,
    format_errs: &[String],
    format: Option<TextFormat>,
) -> Result<(), MessengerError> {
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
    is_error_in_chosen_inline(messenger, inline_message_id, &text, format).await
}

pub async fn is_error_in_inline_query(messenger: &(impl MessengerPort + ?Sized), query_id: &str, text: &str) -> Result<(), MessengerError> {
    messenger.answer_inline_error(AnswerInlineErrorRequest { query_id, text }).await
}

pub async fn delete(messenger: &(impl MessengerPort + ?Sized), chat_id: i64, message_id: i64) -> Result<(), MessengerError> {
    messenger.delete_message(DeleteMessageRequest { chat_id, message_id }).await
}
