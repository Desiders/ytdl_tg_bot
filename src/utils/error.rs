use crate::{
    entities::{ParseRangeError, ParseSectionError},
    errors::ErrorKind,
    interactors::{download::media, get_media},
    services::ytdl,
};

use std::{borrow::Cow, fmt::Debug, fmt::Write, iter};
use telers::errors::SessionErrorKind;

pub fn format_error_report(err: &(impl std::error::Error + ?Sized)) -> String {
    let mut output = String::new();
    write!(&mut output, "{err}").unwrap();

    if let Some(cause) = err.source() {
        write!(&mut output, ". Caused by:").unwrap();
        for (i, err) in iter::successors(Some(cause), |err| err.source()).enumerate() {
            write!(&mut output, " {i}: {err}").unwrap();
        }
    }

    output
}

pub trait FormatErrorToMessage {
    fn format(&self, token: &str) -> Cow<'static, str>;
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

impl FormatErrorToMessage for ytdl::GetInfoErrorKind {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for ytdl::DownloadErrorKind {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
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
        }
    }
}

impl<E> FormatErrorToMessage for ErrorKind<E>
where
    E: std::error::Error + Debug,
{
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}
