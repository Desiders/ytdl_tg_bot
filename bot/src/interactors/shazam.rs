use std::{fmt::Write as _, sync::Arc};

use rust_i18n::t;
use telers::{
    errors::HandlerError,
    utils::text::{html_quote, html_text_link},
};
use tracing::{error, info, instrument};
use url::Url;

use crate::{
    entities::{ChatConfig, Params},
    handlers_utils::progress,
    interactors::{
        enqueue_download::{EnqueueCommandDownload, EnqueueCommandInput},
        Interactor,
    },
    locale::Locale,
    services::{
        file_download::TelegramFileDownloader,
        get_media::{SearchMediaInfo, SearchMediaInfoInput},
        messenger::{EditTarget, EditTextRequest, MessengerPort, TextFormat},
        node_router::{recognize_song, NodeRouter, RecognizedSong},
    },
    utils::ErrorFormatter,
    value_objects::MediaType,
};

/// Telegram voice/short-audio clips are small; reject anything larger so a stray big upload does
/// not get pulled through the bot and node.
const MAX_AUDIO_SIZE: i64 = 25 * 1024 * 1024;

pub struct Shazam<Messenger> {
    error_formatter: Arc<ErrorFormatter>,
    messenger: Arc<Messenger>,
    file_downloader: Arc<TelegramFileDownloader>,
    node_router: Arc<NodeRouter>,
    search_media: Arc<SearchMediaInfo>,
    enqueue_download: Arc<EnqueueCommandDownload<Messenger>>,
}

impl<Messenger> Shazam<Messenger> {
    #[must_use]
    pub const fn new(
        error_formatter: Arc<ErrorFormatter>,
        messenger: Arc<Messenger>,
        file_downloader: Arc<TelegramFileDownloader>,
        node_router: Arc<NodeRouter>,
        search_media: Arc<SearchMediaInfo>,
        enqueue_download: Arc<EnqueueCommandDownload<Messenger>>,
    ) -> Self {
        Self {
            error_formatter,
            messenger,
            file_downloader,
            node_router,
            search_media,
            enqueue_download,
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
        let chat_id = input.chat_id;
        let reply_to_message_id = input.reply_to_message_id;
        let chat_cfg = input.chat_cfg;
        let locale = chat_cfg.map_or(Locale::En, ChatConfig::locale).as_str();

        let Some(file_id) = input.file_id else {
            self.reply(chat_id, reply_to_message_id, &t!("shazam.no_audio", locale = locale))
                .await;
            return Ok(());
        };
        if input.file_size.is_some_and(|size| size > MAX_AUDIO_SIZE) {
            self.reply(chat_id, reply_to_message_id, &t!("shazam.too_large", locale = locale))
                .await;
            return Ok(());
        }

        let placeholder = progress::new(
            self.messenger.as_ref(),
            &t!("shazam.recognizing", locale = locale),
            chat_id,
            Some(reply_to_message_id),
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

        let (text, song) = self.recognize(&file_id, locale).await;
        if let Err(err) = self
            .messenger
            .edit_text(EditTextRequest {
                target: EditTarget::ChatMessage {
                    chat_id,
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

        if let Some(song) = song {
            self.download_song(&song, chat_id, reply_to_message_id, chat_cfg, placeholder_id, &text)
                .await;
        }

        Ok(())
    }
}

impl<Messenger> Shazam<Messenger>
where
    Messenger: MessengerPort,
{
    /// Downloads the clip and recognizes it, returning the message text to show the user and, on a
    /// successful match, the recognized song so the caller can also fetch its audio.
    async fn recognize(&self, file_id: &str, locale: &str) -> (String, Option<RecognizedSong>) {
        let audio = match self.file_downloader.download(file_id).await {
            Ok(audio) => audio,
            Err(err) => {
                error!(%err, "Download user audio error");
                return (t!("shazam.error", locale = locale).into_owned(), None);
            }
        };

        info!(bytes = audio.len(), "Recognizing user audio");
        match recognize_song(self.node_router.as_ref(), audio).await {
            Ok(song) => {
                let text = format_song(&song, locale);
                (text, Some(song))
            }
            Err(err) if err.is_no_match() => (t!("shazam.no_match", locale = locale).into_owned(), None),
            Err(err) => {
                error!(%err, "Recognize song error");
                (t!("shazam.error", locale = locale).into_owned(), None)
            }
        }
    }

    /// Best-effort: searches YouTube for the recognized track and enqueues a normal audio download so
    /// the song is sent (with the usual download progress). Silently skips on any miss.
    async fn download_song(
        &self,
        song: &RecognizedSong,
        chat_id: i64,
        reply_to_message_id: i64,
        chat_cfg: Option<&ChatConfig>,
        progress_message_id: i64,
        base_text: &str,
    ) {
        let Some(chat_cfg) = chat_cfg else { return };

        let title = song.title.as_deref().unwrap_or_default().trim();
        let artist = song.artist.as_deref().unwrap_or_default().trim();
        let query = format!("{artist} {title}");
        let query = query.trim();
        if query.is_empty() {
            return;
        }

        let results = match self.search_media.execute(SearchMediaInfoInput { text: query }).await {
            Ok(results) => results,
            Err(err) => {
                error!(%err, "Shazam song search error");
                return;
            }
        };
        let Some(first) = results.into_iter().next() else {
            info!(query, "No search results for recognized song");
            return;
        };

        let url = match Url::parse(&format!("https://www.youtube.com/watch?v={}", first.id)) {
            Ok(url) => url,
            Err(err) => {
                error!(%err, "Build YouTube URL error");
                return;
            }
        };

        // Build the final caption: the metadata (whose last line is the Shazam link, if any) plus the
        // source link on the same line, so both links read as one `Shazam | Link` row. The source link
        // is baked in here, so the messenger must not append its own (`link_is_visible: false`).
        let source_link = html_text_link("Link", &url);
        let caption = if song.url.as_deref().map(str::trim).is_some_and(|url| !url.is_empty()) {
            format!("{base_text} | {source_link}")
        } else {
            format!("{base_text}\n{source_link}")
        };

        let params = Params::default();
        if let Err(err) = self
            .enqueue_download
            .execute(EnqueueCommandInput {
                media_type: MediaType::Audio,
                chat_id,
                message_id: reply_to_message_id,
                url: &url,
                params: &params,
                chat_cfg,
                link_is_visible: false,
                progress_message_id: Some(progress_message_id),
                base_text: Some(&caption),
            })
            .await
        {
            error!(%err, "Enqueue shazam download error");
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
