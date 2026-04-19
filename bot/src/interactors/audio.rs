use std::{
    str::FromStr as _,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use telers::{
    errors::HandlerError,
    utils::text::{html_expandable_blockquote, html_quote},
};
use tracing::{debug, error, instrument, warn};
use url::Url;

use crate::{
    config::Config,
    database::TxManager,
    entities::{language::Language, ChatConfig, Domains, MediaInPlaylist, Params, Range, Sections},
    handlers_utils::progress,
    interactors::Interactor,
    services::{
        download::media,
        downloaded_media,
        get_media::{
            self,
            GetMediaByURLKind::{Empty, Playlist, SingleCached},
        },
        messenger::{MessengerPort, TextFormat},
        send_media,
    },
    utils::{format_error_report, ErrorMessageFormatter},
};

pub struct Download<Messenger> {
    pub cfg: Arc<Config>,
    pub error_formatter: Arc<ErrorMessageFormatter>,
    pub messenger: Arc<Messenger>,
    pub get_media: Arc<get_media::GetAudioByURL>,
    pub download_playlist: Arc<media::DownloadAudioPlaylist>,
    pub upload_media: Arc<send_media::upload::SendAudio<Messenger>>,
    pub send_media_by_id: Arc<send_media::id::SendAudio<Messenger>>,
    pub send_playlist: Arc<send_media::id::SendAudioPlaylist<Messenger>>,
    pub add_downloaded_media: Arc<downloaded_media::AddAudio>,
}

pub struct DownloadInput<'a> {
    pub message_id: i64,
    pub chat_id: i64,
    pub params: &'a Params,
    pub url: &'a Url,
    pub chat_cfg: &'a ChatConfig,
    pub tx_manager: &'a mut TxManager,
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

        let progress_message = progress::new(
            self.messenger.as_ref(),
            "🔍 Preparing download...",
            input.chat_id,
            Some(input.message_id),
            None,
        )
        .await?;
        let progress_message_id = progress_message.message_id;

        let playlist_range = match input.params.0.get("items") {
            Some(raw_value) => match Range::from_str(raw_value) {
                Ok(val) => val,
                Err(err) => {
                    error!(%err, "Parse range error");
                    let text = format!(
                        "Sorry, an error to parse range\n{}",
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
                        "Sorry, an error to parse sections\n{}",
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

        match self
            .get_media
            .execute(get_media::GetMediaByURLInput {
                url: input.url,
                playlist_range: &playlist_range,
                cache_search: input.url.as_str(),
                domain: input.url.domain(),
                audio_language: &audio_language,
                sections: sections.as_ref(),
                tx_manager: input.tx_manager,
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
                        link_is_visible: input.chat_cfg.link_is_visible,
                    })
                    .await
                {
                    error!(err = format_error_report(&err), "Send error");
                    let text = format!(
                        "Sorry, an error to send media\n{}",
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
                } else {
                    let _ = progress::delete(self.messenger.as_ref(), input.chat_id, progress_message_id).await;
                }
            }
            Ok(Playlist { cached, uncached }) => {
                let mut send_err = None;
                let mut download_errs = vec![];
                let (cached_len, uncached_len) = (cached.len(), uncached.len());
                let mut downloaded_playlist = Vec::with_capacity(cached_len + uncached_len);
                downloaded_playlist.extend(cached);
                let (download_input, mut media_receiver, mut errs_receiver, mut progress_receiver) =
                    media::DownloadMediaPlaylistInput::new_with_progress(input.url, uncached, sections.as_ref());

                let downloaded_media_count = AtomicUsize::new(cached_len);
                tokio::join!(
                    async {
                        while let Some((media_for_upload, media, _format)) = media_receiver.recv().await {
                            let file_id = match self
                                .upload_media
                                .execute(send_media::upload::SendAudioInput {
                                    chat_id: self.cfg.chat.receiver_chat_id,
                                    reply_to_message_id: Some(input.message_id),
                                    media_for_upload,
                                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                                    title: media.title.as_deref(),
                                    performer: media.uploader.as_deref(),
                                    duration: media.duration.map(|val| val as i64),
                                    with_delete: true,
                                    webpage_url: &media.webpage_url,
                                    link_is_visible: true,
                                })
                                .await
                            {
                                Ok(val) => val,
                                Err(err) => {
                                    error!(err = format_error_report(&err), "Send error");
                                    send_err = Some(html_quote(self.error_formatter.format(&err).as_ref()));
                                    continue;
                                }
                            };

                            downloaded_playlist.push(MediaInPlaylist {
                                file_id: file_id.clone().into(),
                                playlist_index: media.playlist_index,
                                webpage_url: Some(media.webpage_url.clone()),
                            });

                            if let Err(err) = self
                                .add_downloaded_media
                                .execute(downloaded_media::AddMediaInput {
                                    file_id: file_id.into(),
                                    id: media.id.clone(),
                                    display_id: media.display_id.clone(),
                                    domain: media.webpage_url.host_str().map(ToOwned::to_owned),
                                    audio_language: audio_language.clone(),
                                    sections: sections.clone(),
                                    tx_manager: input.tx_manager,
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
                                    )
                                    .await
                                    .is_err()
                                    {
                                        break;
                                    }
                                }
                                media::DownloadProgressEvent::Finished => {
                                    let _ = progress::is_sending(self.messenger.as_ref(), input.chat_id, progress_message_id).await;
                                }
                            }
                        }
                    },
                    async {
                        if let Err(err) = self.download_playlist.execute(download_input).await {
                            error!(%err, "Download error");
                            let text = format!(
                                "Sorry, an error to download playlist\n{}",
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
                )
                .await;

                downloaded_playlist.sort_by_key(|val| val.playlist_index);
                if let Err(err) = self
                    .send_playlist
                    .execute(send_media::id::SendPlaylistInput {
                        chat_id: input.chat_id,
                        reply_to_message_id: Some(input.message_id),
                        playlist: downloaded_playlist,
                        link_is_visible: input.chat_cfg.link_is_visible,
                    })
                    .await
                {
                    error!(%err, "Send error");
                    let text = format!(
                        "Sorry, an error to send playlist\n{}",
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
                } else if errs.is_empty() {
                    let _ = progress::delete(self.messenger.as_ref(), input.chat_id, progress_message_id).await;
                }
            }
            Ok(Empty) => {
                warn!("Empty playlist");
                let _ = progress::is_error_in_progress(
                    self.messenger.as_ref(),
                    input.chat_id,
                    input.message_id,
                    "Playlist is empty",
                    Some(TextFormat::Html),
                )
                .await;
            }
            Err(err) => {
                error!(err = format_error_report(&err), "Get error");
                let text = format!(
                    "Sorry, an error to get info\n{}",
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

pub struct Random<Messenger> {
    pub get_media: Arc<downloaded_media::GetRandomAudio>,
    pub send_playlist: Arc<send_media::id::SendAudioPlaylist<Messenger>>,
}

pub struct RandomInput<'a> {
    pub message_id: i64,
    pub chat_id: i64,
    pub params: &'a Params,
    pub tx_manager: &'a mut TxManager,
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
                tx_manager: input.tx_manager,
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
                    })
                    .await
                {
                    error!(%err, "Send error");
                }
            }
            Err(err) => {
                error!(err = format_error_report(&err), "Get error");
            }
        }

        Ok(())
    }
}
