use rust_i18n::t;
use std::sync::Arc;
use telers::{
    errors::HandlerError,
    utils::text::{html_expandable_blockquote, html_quote},
};
use tracing::{debug, error, instrument, warn};
use url::Url;

use crate::{
    config::Config,
    entities::{language::Language, ChatConfig, MediaInPlaylist, Params, Range},
    handlers_utils::progress,
    interactors::Interactor,
    services::{
        downloaded_media,
        get_media::{
            self,
            GetMediaByURLKind::{Empty, Playlist, SingleCached},
        },
        messenger::{MessengerPort, TextFormat},
        send_media,
    },
    utils::ErrorFormatter,
};

pub struct Download<Messenger> {
    cfg: Arc<Config>,
    error_formatter: Arc<ErrorFormatter>,
    messenger: Arc<Messenger>,
    get_media: Arc<get_media::GetPhotoByURL>,
    upload_media: Arc<send_media::upload::SendPhotoUrl<Messenger>>,
    send_media_by_id: Arc<send_media::id::SendPhoto<Messenger>>,
    send_playlist: Arc<send_media::id::SendPhotoPlaylist<Messenger>>,
    add_downloaded_media: Arc<downloaded_media::AddPhoto>,
}

impl<Messenger> Download<Messenger> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        cfg: Arc<Config>,
        error_formatter: Arc<ErrorFormatter>,
        messenger: Arc<Messenger>,
        get_media: Arc<get_media::GetPhotoByURL>,
        upload_media: Arc<send_media::upload::SendPhotoUrl<Messenger>>,
        send_media_by_id: Arc<send_media::id::SendPhoto<Messenger>>,
        send_playlist: Arc<send_media::id::SendPhotoPlaylist<Messenger>>,
        add_downloaded_media: Arc<downloaded_media::AddPhoto>,
    ) -> Self {
        Self {
            cfg,
            error_formatter,
            messenger,
            get_media,
            upload_media,
            send_media_by_id,
            send_playlist,
            add_downloaded_media,
        }
    }
}

pub struct DownloadInput<'a> {
    pub message_id: i64,
    pub chat_id: i64,
    pub params: &'a Params,
    pub url: &'a Url,
    pub chat_cfg: &'a ChatConfig,
    pub link_is_visible: bool,
}

impl<Messenger> Interactor<DownloadInput<'_>> for &Download<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(message_id = input.message_id, url = input.url.as_str(), ?input.params))]
    async fn execute(self, input: DownloadInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Got url");
        let locale = input.chat_cfg.locale();

        let progress_message = match progress::new(
            self.messenger.as_ref(),
            t!("download.preparing", locale = locale.as_str()).as_ref(),
            input.chat_id,
            Some(input.message_id),
            None,
        )
        .await
        {
            Ok(progress_message) => progress_message,
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Send progress error");
                return Ok(());
            }
        };
        let progress_message_id = progress_message.message_id;

        let playlist_range = match input.params.0.get("items") {
            Some(raw_value) => match raw_value.parse::<Range>() {
                Ok(value) => value,
                Err(err) => {
                    error!(%err, "Parse range error");
                    let text = format!(
                        "{}\n{}",
                        t!("download.error_parse_range", locale = locale.as_str()),
                        html_expandable_blockquote(html_quote(self.error_formatter.format(&err).as_ref()))
                    );
                    let _ = progress::is_error_in_progress(
                        self.messenger.as_ref(),
                        input.chat_id,
                        progress_message_id,
                        &text,
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
            },
            None => Range::default(),
        };
        let overwrite_cache = input.params.get_bool("overwrite");

        match self
            .get_media
            .execute(get_media::GetMediaByURLInput {
                url: input.url,
                playlist_range: &playlist_range,
                cache_search: input.url.as_str(),
                domain: input.url.domain(),
                audio_language: &Language::default(),
                sections: None,
                overwrite_cache,
            })
            .await
        {
            Ok(SingleCached(file_id)) => {
                if let Err(err) = self
                    .send_media_by_id
                    .execute(send_media::id::SendMediaInput {
                        chat_id: input.chat_id,
                        reply_to_message_id: Some(input.message_id),
                        id: &file_id,
                        webpage_url: Some(input.url),
                        link_is_visible: input.link_is_visible,
                        caption: None,
                    })
                    .await
                {
                    let err = self.error_formatter.format(&err);
                    error!(%err, "Send error");
                    let text = format!(
                        "{}\n{}",
                        t!("download.error_send_media", locale = locale.as_str()),
                        html_expandable_blockquote(html_quote(err.as_ref()))
                    );
                    let _ = progress::is_error_in_progress(
                        self.messenger.as_ref(),
                        input.chat_id,
                        progress_message_id,
                        &text,
                        Some(TextFormat::Html),
                    )
                    .await;
                } else {
                    let _ = progress::delete(self.messenger.as_ref(), input.chat_id, progress_message_id).await;
                }
            }
            Ok(Playlist { cached, uncached }) => {
                let mut send_err = None;
                let (cached_len, uncached_len) = (cached.len(), uncached.len());
                let mut downloaded_playlist = Vec::with_capacity(cached_len + uncached_len);
                downloaded_playlist.extend(cached);

                let _ = progress::is_sending(
                    self.messenger.as_ref(),
                    input.chat_id,
                    progress_message_id,
                    input.chat_cfg.locale().as_str(),
                    None,
                )
                .await;
                for (media, _formats) in uncached {
                    let Some(photo_url) = media.direct_url.as_ref() else {
                        send_err = Some(html_quote("Photo URL is missing in downloader response"));
                        continue;
                    };
                    let file_id = match self
                        .upload_media
                        .execute(send_media::upload::SendPhotoUrlInput {
                            chat_id: self.cfg.chat.receiver_chat_id,
                            reply_to_message_id: Some(input.message_id),
                            photo_url,
                            with_delete: true,
                            webpage_url: &media.webpage_url,
                            link_is_visible: true,
                        })
                        .await
                    {
                        Ok(val) => val,
                        Err(err) => {
                            let err = self.error_formatter.format(&err);
                            error!(%err, "Send error");
                            send_err = Some(html_quote(err.as_ref()));
                            continue;
                        }
                    };

                    downloaded_playlist.push(MediaInPlaylist {
                        file_id: file_id.clone(),
                        playlist_index: media.playlist_index,
                        webpage_url: Some(media.webpage_url.clone()),
                    });

                    if let Err(err) = self
                        .add_downloaded_media
                        .execute(downloaded_media::AddMediaInput {
                            file_id,
                            id: media.id.clone(),
                            display_id: media.display_id.clone(),
                            domain: media.webpage_url.host_str().map(ToOwned::to_owned),
                            audio_language: Language::default(),
                            sections: None,
                            overwrite_cache,
                        })
                        .await
                    {
                        error!(%err, "Add error");
                    }
                }

                let errs = send_err.map(|err| vec![err]).into_iter().collect::<Vec<_>>();
                let media_to_send_count = downloaded_playlist.len();
                let _ = progress::is_errors_if_exist(
                    self.messenger.as_ref(),
                    input.chat_id,
                    progress_message_id,
                    &errs,
                    media_to_send_count,
                    input.chat_cfg.locale().as_str(),
                    None,
                )
                .await;

                downloaded_playlist.sort_by_key(|item| item.playlist_index);
                if let Err(err) = self
                    .send_playlist
                    .execute(send_media::id::SendPlaylistInput {
                        chat_id: input.chat_id,
                        reply_to_message_id: Some(input.message_id),
                        playlist: downloaded_playlist,
                        link_is_visible: input.link_is_visible,
                        caption: None,
                    })
                    .await
                {
                    let err = self.error_formatter.format(&err);
                    error!(%err, "Send playlist error");
                    let text = format!(
                        "{}\n{}",
                        t!("download.error_send_playlist", locale = locale.as_str()),
                        html_expandable_blockquote(html_quote(err.as_ref()))
                    );
                    let _ = progress::is_error_in_progress(
                        self.messenger.as_ref(),
                        input.chat_id,
                        progress_message_id,
                        &text,
                        Some(TextFormat::Html),
                    )
                    .await;
                } else if errs.is_empty() {
                    // Keep the progress message when there were per-photo errors: sending an empty
                    // playlist "succeeds" as a no-op, and deleting here would wipe the error we just
                    // showed via `is_errors_if_exist`.
                    let _ = progress::delete(self.messenger.as_ref(), input.chat_id, progress_message_id).await;
                }
            }
            Ok(Empty) => {
                warn!("No media");
                let _ = progress::is_error_in_progress(
                    self.messenger.as_ref(),
                    input.chat_id,
                    progress_message_id,
                    &t!("download.no_media_found", locale = locale.as_str()),
                    None,
                )
                .await;
            }
            Err(err) => {
                let err = self.error_formatter.format(&err);
                error!(%err, "Get media error");
                let text = format!(
                    "{}\n{}",
                    t!("download.error_get_media", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(err.as_ref()))
                );
                let _ = progress::is_error_in_progress(
                    self.messenger.as_ref(),
                    input.chat_id,
                    progress_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
                .await;
            }
        }

        Ok(())
    }
}
