use crate::{
    entities::{ParseRangeError, ParseSectionError},
    errors::ErrorKind,
    services::{download::media, get_media, messenger::MessengerError, node_router, yt_toolkit},
};

use std::{borrow::Cow, fmt::Debug, fmt::Write};
use telers::errors::SessionErrorKind;

fn format_error_report(err: &(impl std::error::Error + ?Sized)) -> String {
    let mut output = String::new();
    write!(&mut output, "{err}").unwrap();

    if let Some(cause) = err.source() {
        write!(&mut output, ". Caused by:").unwrap();
        let mut cause = Some(cause);
        let mut index = 0;
        while let Some(err) = cause {
            write!(&mut output, " {index}: {err}").unwrap();
            cause = err.source();
            index += 1;
        }
    }

    output
}

fn redact_token(message: impl Into<String>, token: &str) -> String {
    let message = message.into();
    let mut redacted = String::with_capacity(message.len());
    let mut rest = message.as_str();

    while let Some(marker_index) = rest.find("/bot") {
        let token_start = marker_index + "/bot".len();
        let Some(token_end_offset) = rest[token_start..].find('/') else {
            break;
        };
        let token_end = token_start + token_end_offset;

        redacted.push_str(&rest[..token_start]);
        redacted.push_str("...");
        rest = &rest[token_end..];
    }

    redacted.push_str(rest);
    if token.is_empty() {
        redacted
    } else {
        redacted.replace(token, "...")
    }
}

pub trait FormatErrorToMessage {
    fn format(&self, token: &str) -> Cow<'static, str>;
}

#[derive(Clone)]
pub struct ErrorFormatter {
    token: Box<str>,
}

impl ErrorFormatter {
    pub fn new(token: impl Into<Box<str>>) -> Self {
        Self { token: token.into() }
    }

    pub fn format(&self, err: &(impl FormatErrorToMessage + ?Sized)) -> Cow<'static, str> {
        err.format(&self.token)
    }
}

impl FormatErrorToMessage for SessionErrorKind {
    fn format(&self, token: &str) -> Cow<'static, str> {
        match self {
            SessionErrorKind::Client(err) => Cow::Owned(err.to_string().replace(token, "...")),
            SessionErrorKind::Parse(err) => Cow::Owned(err.to_string()),
            SessionErrorKind::Telegram(err) => Cow::Owned(err.to_string()),
        }
    }
}

impl FormatErrorToMessage for media::DownloadMediaErrorKind {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for media::DownloadMediaPlaylistErrorKind {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for node_router::DownloadErrorKind {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for get_media::GetInfoErrorKind {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for yt_toolkit::GetVideoInfoErrorKind {
    fn format(&self, token: &str) -> Cow<'static, str> {
        Cow::Owned(redact_token(format_error_report(self), token))
    }
}

impl FormatErrorToMessage for yt_toolkit::SearchVideoErrorKind {
    fn format(&self, token: &str) -> Cow<'static, str> {
        Cow::Owned(redact_token(format_error_report(self), token))
    }
}

impl FormatErrorToMessage for ParseRangeError {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for ParseSectionError {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for get_media::GetMediaByURLErrorKind {
    fn format(&self, token: &str) -> Cow<'static, str> {
        match self {
            get_media::GetMediaByURLErrorKind::GetInfo(err) => err.format(token),
            get_media::GetMediaByURLErrorKind::Database(err) => err.format(token),
            get_media::GetMediaByURLErrorKind::NodeUnavailable => Cow::Owned(self.to_string()),
        }
    }
}

impl FormatErrorToMessage for MessengerError {
    fn format(&self, token: &str) -> Cow<'static, str> {
        Cow::Owned(redact_token(self.to_string(), token))
    }
}

impl<E> FormatErrorToMessage for ErrorKind<E>
where
    E: std::error::Error + Debug,
{
    fn format(&self, token: &str) -> Cow<'static, str> {
        Cow::Owned(redact_token(format_error_report(self), token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messenger_error_message_redacts_bot_token() {
        let token = "123:SECRET";
        let err = MessengerError::new(format!(
            "error sending request for url (http://telegram-bot-api:8081/bot{token}/sendVideo)"
        ));

        let formatter = ErrorFormatter::new(token);
        let message = formatter.format(&err);

        assert!(!message.contains(token));
        assert!(message.contains("bot.../sendVideo"));
    }
}
