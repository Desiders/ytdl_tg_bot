use crate::{
    entities::ParseRangeError,
    errors::FormatNotFound,
    interactors::download::{
        DownloadAudioErrorKind, DownloadAudioPlaylistErrorKind, DownloadVideoErrorKind, DownloadVideoPlaylistErrorKind,
    },
    services::ytdl,
};

use std::{borrow::Cow, fmt::Write, iter};
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

impl FormatErrorToMessage for DownloadAudioErrorKind {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for DownloadAudioPlaylistErrorKind {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for DownloadVideoErrorKind {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for DownloadVideoPlaylistErrorKind {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for FormatNotFound {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for ytdl::Error {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl FormatErrorToMessage for ParseRangeError {
    fn format(&self, _token: &str) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}
