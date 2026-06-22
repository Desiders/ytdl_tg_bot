use std::{
    str::FromStr as _,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use rust_i18n::t;
use telers::{
    errors::HandlerError,
    utils::text::{html_expandable_blockquote, html_quote},
};
use tracing::{debug, error, instrument, warn};
use url::Url;

use crate::{
    config::Config,
    entities::{language::Language, ChatConfig, Domains, MediaInPlaylist, Params, Range, Sections},
    handlers_utils::progress,
    interactors::{auto::FulfillCtx, Interactor},
    services::{
        download::media,
        downloaded_media,
        get_media::{
            self,
            GetMediaByURLKind::{self, Empty, Playlist, SingleCached},
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
    get_media: Arc<get_media::GetVideoByURL>,
    playlist_downloader: Arc<media::DownloadVideoPlaylist>,
    upload_media: Arc<send_media::upload::SendVideo<Messenger>>,
    send_media_by_id: Arc<send_media::id::SendVideo<Messenger>>,
    send_playlist: Arc<send_media::id::SendVideoPlaylist<Messenger>>,
    add_downloaded_media: Arc<downloaded_media::AddVideo>,
}

impl<Messenger> Download<Messenger> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        cfg: Arc<Config>,
        error_formatter: Arc<ErrorFormatter>,
        messenger: Arc<Messenger>,
        get_media: Arc<get_media::GetVideoByURL>,
        playlist_downloader: Arc<media::DownloadVideoPlaylist>,
        upload_media: Arc<send_media::upload::SendVideo<Messenger>>,
        send_media_by_id: Arc<send_media::id::SendVideo<Messenger>>,
        send_playlist: Arc<send_media::id::SendVideoPlaylist<Messenger>>,
        add_downloaded_media: Arc<downloaded_media::AddVideo>,
    ) -> Self {
        Self {
            cfg,
            error_formatter,
            messenger,
            get_media,
            playlist_downloader,
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
    pub prefetched: Option<GetMediaByURLKind>,
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
            Some(raw_value) => match Range::from_str(raw_value) {
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

        let sections = match input.params.0.get("crop") {
            Some(raw_value) => Some(match Sections::from_str(raw_value) {
                Ok(val) => val,
                Err(err) => {
                    error!(%err, "Parse sections error");
                    let text = format!(
                        "{}\n{}",
                        t!("download.error_parse_sections", locale = locale.as_str()),
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
            }),
            None => None,
        };

        let audio_language = match input.params.0.get("lang") {
            Some(raw_value) => Language::from_str(raw_value).unwrap(),
            None => Language::default(),
        };
        let overwrite_cache = input.params.get_bool("overwrite");

        let result = match input.prefetched {
            Some(result) => Ok(result),
            None => {
                self.get_media
                    .execute(get_media::GetMediaByURLInput {
                        url: input.url,
                        playlist_range: &playlist_range,
                        cache_search: input.url.as_str(),
                        domain: input.url.domain(),
                        audio_language: &audio_language,
                        sections: sections.as_ref(),
                        overwrite_cache,
                    })
                    .await
            }
        };
        match result {
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
                let mut download_errs: Vec<Vec<_>> = vec![];
                let (cached_len, uncached_len) = (cached.len(), uncached.len());
                let mut downloaded_playlist = Vec::with_capacity(cached_len + uncached_len);
                downloaded_playlist.extend(cached);
                let (download_input, mut media_receiver, mut errs_receiver, mut progress_receiver) =
                    media::DownloadMediaPlaylistInput::new_with_progress(input.url, uncached, sections.as_ref());

                let downloaded_media_count = AtomicUsize::new(cached_len);
                tokio::join!(
                    async {
                        while let Some((media_for_upload, media, format, duration)) = media_receiver.recv().await {
                            let file_id = match self
                                .upload_media
                                .execute(send_media::upload::SendVideoInput {
                                    chat_id: self.cfg.chat.receiver_chat_id,
                                    reply_to_message_id: Some(input.message_id),
                                    media_for_upload,
                                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                                    width: format.width,
                                    height: format.height,
                                    duration,
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
                                    audio_language: audio_language.clone(),
                                    sections: sections.clone(),
                                    overwrite_cache,
                                })
                                .await
                            {
                                error!(%err, "Add error");
                            }

                            downloaded_media_count.fetch_add(1, Ordering::SeqCst);
                        }
                    },
                    async {
                        while let Some(errs) = errs_receiver.recv().await {
                            download_errs.push(
                                errs.into_iter()
                                    .map(|err| html_quote(self.error_formatter.format(&err).as_ref()))
                                    .collect(),
                            );
                        }
                    },
                    async {
                        while let Some(event) = progress_receiver.recv().await {
                            match event {
                                media::DownloadProgressEvent::Progress(progress_str) => {
                                    if progress::is_downloading_with_progress(
                                        self.messenger.as_ref(),
                                        input.chat_id,
                                        progress_message_id,
                                        progress_str,
                                        downloaded_media_count.load(Ordering::SeqCst),
                                        cached_len + uncached_len,
                                        input.chat_cfg.locale().as_str(),
                                        None,
                                    )
                                    .await
                                    .is_err()
                                    {
                                        break;
                                    }
                                }
                                media::DownloadProgressEvent::Finished => {
                                    let _ = progress::is_sending(
                                        self.messenger.as_ref(),
                                        input.chat_id,
                                        progress_message_id,
                                        input.chat_cfg.locale().as_str(),
                                        None,
                                    )
                                    .await;
                                }
                            }
                        }
                    },
                    async {
                        if let Err(err) = self.playlist_downloader.execute(download_input).await {
                            error!(%err, "Download error");
                            let text = format!(
                                "{}\n{}",
                                t!("download.error_download_playlist", locale = locale.as_str()),
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
                        }
                    }
                );

                let errs = download_errs.into_iter().chain(send_err.map(|err| vec![err])).collect::<Vec<_>>();
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

                downloaded_playlist.sort_by_key(|val| val.playlist_index);
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
                    error!(%err, "Send error");
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
                    let _ = progress::delete(self.messenger.as_ref(), input.chat_id, progress_message_id).await;
                }
            }
            Ok(Empty) => {
                warn!("Empty playlist");
                let _ = progress::is_error_in_progress(
                    self.messenger.as_ref(),
                    input.chat_id,
                    progress_message_id,
                    &t!("download.playlist_empty", locale = locale.as_str()),
                    Some(TextFormat::Html),
                )
                .await;
            }
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Get error");
                let text = format!(
                    "{}\n{}",
                    t!("download.error_get_info", locale = locale.as_str()),
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
            }
        }

        Ok(())
    }
}

pub struct DownloadQuiet<Messenger> {
    cfg: Arc<Config>,
    error_formatter: Arc<ErrorFormatter>,
    get_media: Arc<get_media::GetVideoByURL>,
    playlist_downloader: Arc<media::DownloadVideoPlaylist>,
    upload_media: Arc<send_media::upload::SendVideo<Messenger>>,
    send_media_by_id: Arc<send_media::id::SendVideo<Messenger>>,
    send_playlist: Arc<send_media::id::SendVideoPlaylist<Messenger>>,
    add_downloaded_media: Arc<downloaded_media::AddVideo>,
}

impl<Messenger> DownloadQuiet<Messenger> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        cfg: Arc<Config>,
        error_formatter: Arc<ErrorFormatter>,
        get_media: Arc<get_media::GetVideoByURL>,
        playlist_downloader: Arc<media::DownloadVideoPlaylist>,
        upload_media: Arc<send_media::upload::SendVideo<Messenger>>,
        send_media_by_id: Arc<send_media::id::SendVideo<Messenger>>,
        send_playlist: Arc<send_media::id::SendVideoPlaylist<Messenger>>,
        add_downloaded_media: Arc<downloaded_media::AddVideo>,
    ) -> Self {
        Self {
            cfg,
            error_formatter,
            get_media,
            playlist_downloader,
            upload_media,
            send_media_by_id,
            send_playlist,
            add_downloaded_media,
        }
    }
}

pub struct DownloadQuietInput<'a> {
    pub message_id: i64,
    pub chat_id: i64,
    pub params: &'a Params,
    pub url: &'a Url,
    pub link_is_visible: bool,
}

impl<Messenger> Interactor<DownloadQuietInput<'_>> for &DownloadQuiet<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(message_id = input.message_id, url = input.url.as_str(), ?input.params))]
    async fn execute(self, input: DownloadQuietInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Got url");

        let playlist_range = match input.params.0.get("items") {
            Some(raw_value) => match Range::from_str(raw_value) {
                Ok(val) => val,
                Err(err) => {
                    error!(%err, "Parse range error");
                    return Ok(());
                }
            },
            None => Range::default(),
        };
        let sections = match input.params.0.get("crop") {
            Some(raw_value) => Some(match Sections::from_str(raw_value) {
                Ok(val) => val,
                Err(err) => {
                    error!(%err, "Parse sections error");
                    return Ok(());
                }
            }),
            None => None,
        };
        let audio_language = match input.params.0.get("lang") {
            Some(raw_value) => Language::from_str(raw_value).unwrap(),
            None => Language::default(),
        };
        let overwrite_cache = input.params.get_bool("overwrite");

        let ctx = FulfillCtx {
            chat_id: input.chat_id,
            message_id: input.message_id,
            url: input.url,
            link_is_visible: input.link_is_visible,
            sections: sections.as_ref(),
            audio_language: &audio_language,
            overwrite_cache,
        };

        match self
            .get_media
            .execute(get_media::GetMediaByURLInput {
                url: input.url,
                playlist_range: &playlist_range,
                cache_search: input.url.as_str(),
                domain: input.url.domain(),
                audio_language: &audio_language,
                sections: sections.as_ref(),
                overwrite_cache,
            })
            .await
        {
            Ok(result) => self.fulfill(result, &ctx).await,
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Get error");
            }
        }

        Ok(())
    }
}

impl<Messenger> DownloadQuiet<Messenger>
where
    Messenger: MessengerPort,
{
    // Silent video download+send for an already-resolved result (shared with the auto path).
    pub async fn fulfill(&self, result: GetMediaByURLKind, ctx: &FulfillCtx<'_>) {
        match result {
            SingleCached(file_id) => {
                if let Err(err) = self
                    .send_media_by_id
                    .execute(send_media::id::SendMediaInput {
                        chat_id: ctx.chat_id,
                        reply_to_message_id: Some(ctx.message_id),
                        id: &file_id,
                        webpage_url: Some(ctx.url),
                        link_is_visible: ctx.link_is_visible,
                        caption: None,
                    })
                    .await
                {
                    error!(err = %self.error_formatter.format(&err), "Send error");
                }
            }
            Playlist { cached, uncached } => {
                let mut downloaded_playlist = Vec::with_capacity(cached.len() + uncached.len());
                downloaded_playlist.extend(cached);
                let (download_input, mut media_receiver) = media::DownloadMediaPlaylistInput::new(ctx.url, uncached, ctx.sections);

                tokio::join!(
                    async {
                        while let Some((media_for_upload, media, format, duration)) = media_receiver.recv().await {
                            let file_id = match self
                                .upload_media
                                .execute(send_media::upload::SendVideoInput {
                                    chat_id: self.cfg.chat.receiver_chat_id,
                                    reply_to_message_id: Some(ctx.message_id),
                                    media_for_upload,
                                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                                    width: format.width,
                                    height: format.height,
                                    duration,
                                    with_delete: true,
                                    webpage_url: &media.webpage_url,
                                    link_is_visible: true,
                                })
                                .await
                            {
                                Ok(val) => val,
                                Err(err) => {
                                    error!(err = %self.error_formatter.format(&err), "Send error");
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
                                    audio_language: ctx.audio_language.clone(),
                                    sections: ctx.sections.cloned(),
                                    overwrite_cache: ctx.overwrite_cache,
                                })
                                .await
                            {
                                error!(%err, "Add error");
                            }
                        }
                    },
                    async {
                        if let Err(err) = self.playlist_downloader.execute(download_input).await {
                            error!(%err, "Download error");
                        }
                    }
                );

                downloaded_playlist.sort_by_key(|val| val.playlist_index);
                if let Err(err) = self
                    .send_playlist
                    .execute(send_media::id::SendPlaylistInput {
                        chat_id: ctx.chat_id,
                        reply_to_message_id: Some(ctx.message_id),
                        playlist: downloaded_playlist,
                        link_is_visible: ctx.link_is_visible,
                        caption: None,
                    })
                    .await
                {
                    error!(err = %self.error_formatter.format(&err), "Send error");
                }
            }
            Empty => {
                warn!("Empty playlist");
            }
        }
    }
}

pub struct Random<Messenger> {
    error_formatter: Arc<ErrorFormatter>,
    get_media: Arc<downloaded_media::GetRandomVideo>,
    send_playlist: Arc<send_media::id::SendVideoPlaylist<Messenger>>,
}

impl<Messenger> Random<Messenger> {
    #[must_use]
    pub const fn new(
        error_formatter: Arc<ErrorFormatter>,
        get_media: Arc<downloaded_media::GetRandomVideo>,
        send_playlist: Arc<send_media::id::SendVideoPlaylist<Messenger>>,
    ) -> Self {
        Self {
            error_formatter,
            get_media,
            send_playlist,
        }
    }
}

pub struct RandomInput<'a> {
    pub message_id: i64,
    pub chat_id: i64,
    pub params: &'a Params,
}

impl<Messenger> Interactor<RandomInput<'_>> for &Random<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(message_id = input.message_id))]
    async fn execute(self, input: RandomInput<'_>) -> Result<Self::Output, Self::Err> {
        let domains = input.params.0.get("domains").map(|val| Domains::from_str(val).unwrap());

        match self
            .get_media
            .execute(downloaded_media::GetRandomMediaInput {
                limit: 1,
                domains: domains.as_ref(),
            })
            .await
        {
            Ok(playlist) => {
                if let Err(err) = self
                    .send_playlist
                    .execute(send_media::id::SendPlaylistInput {
                        chat_id: input.chat_id,
                        reply_to_message_id: Some(input.message_id),
                        playlist: playlist
                            .into_iter()
                            .enumerate()
                            .map(|(index, media)| MediaInPlaylist {
                                file_id: media.file_id,
                                playlist_index: index.try_into().unwrap(),
                                webpage_url: None,
                            })
                            .collect(),
                        link_is_visible: false,
                        caption: None,
                    })
                    .await
                {
                    error!(err = %self.error_formatter.format(&err), "Send error");
                }
            }
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Get error");
            }
        }

        Ok(())
    }
}
