use std::{fmt::Write as _, sync::Arc};

use rust_i18n::t;
use telers::{errors::HandlerError, utils::text::html_quote};
use tracing::{error, info, instrument};

use crate::{
    entities::ChatConfig,
    handlers_utils::progress,
    interactors::Interactor,
    locale::Locale,
    services::{
        file_download::TelegramFileDownloader,
        messenger::{EditTarget, EditTextRequest, MessengerPort, TextFormat},
        node_router::{recognize_song, NodeRouter, RecognizedSong},
    },
    utils::ErrorFormatter,
};

/// Telegram voice/short-audio clips are small; reject anything larger so a stray big upload does
/// not get pulled through the bot and node.
const MAX_AUDIO_SIZE: i64 = 25 * 1024 * 1024;

pub struct Shazam<Messenger> {
    error_formatter: Arc<ErrorFormatter>,
    messenger: Arc<Messenger>,
    file_downloader: Arc<TelegramFileDownloader>,
    node_router: Arc<NodeRouter>,
}

impl<Messenger> Shazam<Messenger> {
    #[must_use]
    pub const fn new(
        error_formatter: Arc<ErrorFormatter>,
        messenger: Arc<Messenger>,
        file_downloader: Arc<TelegramFileDownloader>,
        node_router: Arc<NodeRouter>,
    ) -> Self {
        Self {
            error_formatter,
            messenger,
            file_downloader,
            node_router,
        }
    }
}

pub struct ShazamInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: i64,
    pub file_id: Option<String>,
    pub file_size: Option<i64>,
    pub chat_cfg: Option<&'a ChatConfig>,
}

impl<Messenger> Interactor<ShazamInput<'_>> for &Shazam<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(chat_id = input.chat_id))]
    async fn execute(self, input: ShazamInput<'_>) -> Result<Self::Output, Self::Err> {
        let locale = input.chat_cfg.map_or(Locale::En, ChatConfig::locale).as_str();

        let Some(file_id) = input.file_id else {
            self.reply(input.chat_id, input.reply_to_message_id, &t!("shazam.no_audio", locale = locale))
                .await;
            return Ok(());
        };
        if input.file_size.is_some_and(|size| size > MAX_AUDIO_SIZE) {
            self.reply(input.chat_id, input.reply_to_message_id, &t!("shazam.too_large", locale = locale))
                .await;
            return Ok(());
        }

        let placeholder = progress::new(
            self.messenger.as_ref(),
            &t!("shazam.recognizing", locale = locale),
            input.chat_id,
            Some(input.reply_to_message_id),
            Some(TextFormat::Html),
        )
        .await;
        let placeholder_id = match placeholder {
            Ok(sent) => sent.message_id,
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Send error");
                return Ok(());
            }
        };

        let text = self.recognize(&file_id, locale).await;
        if let Err(err) = self
            .messenger
            .edit_text(EditTextRequest {
                target: EditTarget::ChatMessage {
                    chat_id: input.chat_id,
                    message_id: placeholder_id,
                },
                text: &text,
                format: Some(TextFormat::Html),
                disable_link_preview: true,
                clear_inline_keyboard: false,
            })
            .await
        {
            error!(err = %self.error_formatter.format(&err), "Edit error");
        }

        Ok(())
    }
}

impl<Messenger> Shazam<Messenger>
where
    Messenger: MessengerPort,
{
    /// Downloads the clip and recognizes it, returning the message text to show the user.
    async fn recognize(&self, file_id: &str, locale: &str) -> String {
        let audio = match self.file_downloader.download(file_id).await {
            Ok(audio) => audio,
            Err(err) => {
                error!(%err, "Download user audio error");
                return t!("shazam.error", locale = locale).into_owned();
            }
        };

        info!(bytes = audio.len(), "Recognizing user audio");
        match recognize_song(self.node_router.as_ref(), audio).await {
            Ok(song) => format_song(&song, locale),
            Err(err) if err.is_no_match() => t!("shazam.no_match", locale = locale).into_owned(),
            Err(err) => {
                error!(%err, "Recognize song error");
                t!("shazam.error", locale = locale).into_owned()
            }
        }
    }

    async fn reply(&self, chat_id: i64, reply_to_message_id: i64, text: &str) {
        if let Err(err) = progress::new(
            self.messenger.as_ref(),
            text,
            chat_id,
            Some(reply_to_message_id),
            Some(TextFormat::Html),
        )
        .await
        {
            error!(err = %self.error_formatter.format(&err), "Send error");
        }
    }
}

fn format_song(song: &RecognizedSong, locale: &str) -> String {
    let title = song.title.as_deref().unwrap_or_default();
    let artist = song.artist.as_deref().unwrap_or_default();

    let mut body = t!(
        "shazam.result",
        locale = locale,
        title = html_quote(title),
        artist = html_quote(artist),
    )
    .into_owned();

    if let Some(album) = song.album.as_deref().filter(|album| !album.is_empty()) {
        let _ = write!(body, "\n{}", t!("shazam.album", locale = locale, album = html_quote(album)));
    }
    if let Some(url) = song.url.as_deref().filter(|url| !url.is_empty()) {
        let _ = write!(body, "\n{}", t!("shazam.link", locale = locale, url = html_quote(url)));
    }
    body
}
