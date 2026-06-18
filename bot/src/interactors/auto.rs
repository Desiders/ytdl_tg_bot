use std::{str::FromStr as _, sync::Arc};

use telers::errors::HandlerError;
use tracing::{debug, error, instrument, warn};
use url::Url;

use crate::{
    config::Config,
    entities::{language::Language, ChatConfig, Media, MediaFormat, MediaInPlaylist, Params, Range, RawMediaWithFormat, Sections},
    interactors::Interactor,
    services::{
        download::media,
        downloaded_media,
        get_media::{
            self,
            GetMediaByURLKind::{self, Empty, Playlist, SingleCached},
        },
        messenger::MessengerPort,
        send_media,
    },
    utils::ErrorFormatter,
    value_objects::MediaType,
};

// Download context shared by the silent fulfillers, parsed once from the message params.
pub struct FulfillCtx<'a> {
    pub chat_id: i64,
    pub message_id: i64,
    pub url: &'a Url,
    pub link_is_visible: bool,
    pub sections: Option<&'a Sections>,
    pub audio_language: &'a Language,
    pub overwrite_cache: bool,
}

// Silent audio download+send for an already-resolved result.
pub struct AudioFulfiller<Messenger> {
    cfg: Arc<Config>,
    error_formatter: Arc<ErrorFormatter>,
    playlist_downloader: Arc<media::DownloadAudioPlaylist>,
    upload_media: Arc<send_media::upload::SendAudio<Messenger>>,
    send_media_by_id: Arc<send_media::id::SendAudio<Messenger>>,
    send_playlist: Arc<send_media::id::SendAudioPlaylist<Messenger>>,
    add_downloaded_media: Arc<downloaded_media::AddAudio>,
}

impl<Messenger> AudioFulfiller<Messenger> {
    #[must_use]
    pub const fn new(
        cfg: Arc<Config>,
        error_formatter: Arc<ErrorFormatter>,
        playlist_downloader: Arc<media::DownloadAudioPlaylist>,
        upload_media: Arc<send_media::upload::SendAudio<Messenger>>,
        send_media_by_id: Arc<send_media::id::SendAudio<Messenger>>,
        send_playlist: Arc<send_media::id::SendAudioPlaylist<Messenger>>,
        add_downloaded_media: Arc<downloaded_media::AddAudio>,
    ) -> Self {
        Self {
            cfg,
            error_formatter,
            playlist_downloader,
            upload_media,
            send_media_by_id,
            send_playlist,
            add_downloaded_media,
        }
    }
}

impl<Messenger> AudioFulfiller<Messenger>
where
    Messenger: MessengerPort,
{
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
                        while let Some((media_for_upload, media, _format, duration)) = media_receiver.recv().await {
                            let file_id = match self
                                .upload_media
                                .execute(send_media::upload::SendAudioInput {
                                    chat_id: self.cfg.chat.receiver_chat_id,
                                    reply_to_message_id: Some(ctx.message_id),
                                    media_for_upload,
                                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                                    title: media.title.as_deref(),
                                    performer: media.uploader.as_deref(),
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
            Empty => warn!("Empty playlist"),
        }
    }
}

// Silent photo send for an already-resolved result.
pub struct PhotoFulfiller<Messenger> {
    cfg: Arc<Config>,
    error_formatter: Arc<ErrorFormatter>,
    upload_media: Arc<send_media::upload::SendPhotoUrl<Messenger>>,
    send_media_by_id: Arc<send_media::id::SendPhoto<Messenger>>,
    send_playlist: Arc<send_media::id::SendPhotoPlaylist<Messenger>>,
    add_downloaded_media: Arc<downloaded_media::AddPhoto>,
}

impl<Messenger> PhotoFulfiller<Messenger> {
    #[must_use]
    pub const fn new(
        cfg: Arc<Config>,
        error_formatter: Arc<ErrorFormatter>,
        upload_media: Arc<send_media::upload::SendPhotoUrl<Messenger>>,
        send_media_by_id: Arc<send_media::id::SendPhoto<Messenger>>,
        send_playlist: Arc<send_media::id::SendPhotoPlaylist<Messenger>>,
        add_downloaded_media: Arc<downloaded_media::AddPhoto>,
    ) -> Self {
        Self {
            cfg,
            error_formatter,
            upload_media,
            send_media_by_id,
            send_playlist,
            add_downloaded_media,
        }
    }
}

impl<Messenger> PhotoFulfiller<Messenger>
where
    Messenger: MessengerPort,
{
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

                for (media, _formats) in uncached {
                    let Some(photo_url) = media.direct_url.as_ref() else {
                        error!("Photo URL is missing in downloader response");
                        continue;
                    };
                    let file_id = match self
                        .upload_media
                        .execute(send_media::upload::SendPhotoUrlInput {
                            chat_id: self.cfg.chat.receiver_chat_id,
                            reply_to_message_id: Some(ctx.message_id),
                            photo_url,
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
                            audio_language: Language::default(),
                            sections: None,
                            overwrite_cache: ctx.overwrite_cache,
                        })
                        .await
                    {
                        error!(%err, "Add error");
                    }
                }

                downloaded_playlist.sort_by_key(|item| item.playlist_index);
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
            Empty => warn!("No media"),
        }
    }
}

// Silent variant of `Auto`: the same video -> audio -> photo classification, but sends with no
// progress message. Runs in the worker; used for group chats.
pub struct AutoQuiet<Messenger> {
    error_formatter: Arc<ErrorFormatter>,
    get_video: Arc<get_media::GetVideoByURL>,
    get_audio: Arc<get_media::GetAudioByURL>,
    get_photo: Arc<get_media::GetPhotoByURL>,
    video: Arc<super::video::DownloadQuiet<Messenger>>,
    audio: Arc<AudioFulfiller<Messenger>>,
    photo: Arc<PhotoFulfiller<Messenger>>,
}

impl<Messenger> AutoQuiet<Messenger> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        error_formatter: Arc<ErrorFormatter>,
        get_video: Arc<get_media::GetVideoByURL>,
        get_audio: Arc<get_media::GetAudioByURL>,
        get_photo: Arc<get_media::GetPhotoByURL>,
        video: Arc<super::video::DownloadQuiet<Messenger>>,
        audio: Arc<AudioFulfiller<Messenger>>,
        photo: Arc<PhotoFulfiller<Messenger>>,
    ) -> Self {
        Self {
            error_formatter,
            get_video,
            get_audio,
            get_photo,
            video,
            audio,
            photo,
        }
    }
}

pub struct AutoQuietInput<'a> {
    pub message_id: i64,
    pub chat_id: i64,
    pub params: &'a Params,
    pub url: &'a Url,
    pub link_is_visible: bool,
}

impl<Messenger> Interactor<AutoQuietInput<'_>> for &AutoQuiet<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(message_id = input.message_id, url = input.url.as_str(), ?input.params))]
    async fn execute(self, input: AutoQuietInput<'_>) -> Result<Self::Output, Self::Err> {
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

        // Cascade video -> audio -> photo, first success wins. Video is taken only when the probe
        // actually carries a video stream; any other outcome (audio-only, empty, or an error such as
        // Spotify's DRM or an Instagram photo's unsupported-URL) falls through to the next step.
        match self
            .get_video
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
            Ok(SingleCached(file_id)) => {
                self.video.fulfill(SingleCached(file_id), &ctx).await;
                return Ok(());
            }
            Ok(Playlist { cached, uncached }) if has_video_stream(&cached, &uncached) => {
                self.video.fulfill(Playlist { cached, uncached }, &ctx).await;
                return Ok(());
            }
            Ok(_) => {}
            Err(err) => debug!(err = %self.error_formatter.format(&err), "Auto video probe failed"),
        }

        match self
            .get_audio
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
            Ok(Empty) => {}
            Ok(result) => {
                self.audio.fulfill(result, &ctx).await;
                return Ok(());
            }
            Err(err) => debug!(err = %self.error_formatter.format(&err), "Auto audio probe failed"),
        }

        match self
            .get_photo
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
            Ok(Empty) => debug!("Auto found no downloadable media"),
            Ok(result) => self.photo.fulfill(result, &ctx).await,
            Err(err) => debug!(err = %self.error_formatter.format(&err), "Auto found no downloadable media"),
        }

        Ok(())
    }
}

// Video when any uncached format has dimensions or a video container ext (generic/snapsave often
// omits width), or when every entry was already cached as video.
fn has_video_stream(cached: &[MediaInPlaylist], uncached: &[(Media, Vec<(MediaFormat, RawMediaWithFormat)>)]) -> bool {
    if uncached.iter().any(|(_, formats)| {
        formats
            .iter()
            .any(|(format, _)| format.width.is_some() || is_video_ext(&format.ext))
    }) {
        return true;
    }
    uncached.is_empty() && !cached.is_empty()
}

// Default bare-link download: classify the link (video -> audio -> photo), then run that type's
// downloader with its progress message. Runs in the worker; used for private chats.
pub struct Auto<Messenger> {
    get_video: Arc<get_media::GetVideoByURL>,
    get_audio: Arc<get_media::GetAudioByURL>,
    get_photo: Arc<get_media::GetPhotoByURL>,
    video: Arc<super::video::Download<Messenger>>,
    audio: Arc<super::audio::Download<Messenger>>,
    photo: Arc<super::photo::Download<Messenger>>,
}

impl<Messenger> Auto<Messenger> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        get_video: Arc<get_media::GetVideoByURL>,
        get_audio: Arc<get_media::GetAudioByURL>,
        get_photo: Arc<get_media::GetPhotoByURL>,
        video: Arc<super::video::Download<Messenger>>,
        audio: Arc<super::audio::Download<Messenger>>,
        photo: Arc<super::photo::Download<Messenger>>,
    ) -> Self {
        Self {
            get_video,
            get_audio,
            get_photo,
            video,
            audio,
            photo,
        }
    }
}

impl<Messenger> Auto<Messenger> {
    // First-success probe over video -> audio -> photo. Falls back to video so an undownloadable link
    // still surfaces the video extractor's error to the user.
    async fn classify(
        &self,
        url: &Url,
        range: &Range,
        sections: Option<&Sections>,
        audio_language: &Language,
        overwrite_cache: bool,
    ) -> MediaType {
        if let Ok(result) = self
            .get_video
            .execute(get_media::GetMediaByURLInput {
                url,
                playlist_range: range,
                cache_search: url.as_str(),
                domain: url.domain(),
                audio_language,
                sections,
                overwrite_cache,
            })
            .await
        {
            match result {
                SingleCached(_) => return MediaType::Video,
                Playlist { cached, uncached } if has_video_stream(&cached, &uncached) => return MediaType::Video,
                _ => {}
            }
        }
        if let Ok(result) = self
            .get_audio
            .execute(get_media::GetMediaByURLInput {
                url,
                playlist_range: range,
                cache_search: url.as_str(),
                domain: url.domain(),
                audio_language,
                sections,
                overwrite_cache,
            })
            .await
        {
            if !matches!(result, Empty) {
                return MediaType::Audio;
            }
        }
        if let Ok(result) = self
            .get_photo
            .execute(get_media::GetMediaByURLInput {
                url,
                playlist_range: range,
                cache_search: url.as_str(),
                domain: url.domain(),
                audio_language: &Language::default(),
                sections: None,
                overwrite_cache,
            })
            .await
        {
            if !matches!(result, Empty) {
                return MediaType::Photo;
            }
        }
        MediaType::Video
    }
}

pub struct AutoInput<'a> {
    pub message_id: i64,
    pub chat_id: i64,
    pub url: &'a Url,
    pub params: &'a Params,
    pub chat_cfg: &'a ChatConfig,
    pub link_is_visible: bool,
}

impl<Messenger> Interactor<AutoInput<'_>> for &Auto<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(message_id = input.message_id, url = input.url.as_str(), ?input.params))]
    async fn execute(self, input: AutoInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Got url");

        let playlist_range = input
            .params
            .0
            .get("items")
            .and_then(|raw| Range::from_str(raw).ok())
            .unwrap_or_default();
        let sections = input.params.0.get("crop").and_then(|raw| Sections::from_str(raw).ok());
        let audio_language = input
            .params
            .0
            .get("lang")
            .and_then(|raw| Language::from_str(raw).ok())
            .unwrap_or_default();
        let overwrite_cache = input.params.get_bool("overwrite");

        let media_type = self
            .classify(input.url, &playlist_range, sections.as_ref(), &audio_language, overwrite_cache)
            .await;

        match media_type {
            MediaType::Video => {
                self.video
                    .execute(super::video::DownloadInput {
                        message_id: input.message_id,
                        chat_id: input.chat_id,
                        params: input.params,
                        url: input.url,
                        chat_cfg: input.chat_cfg,
                        link_is_visible: input.link_is_visible,
                    })
                    .await
            }
            MediaType::Audio => {
                self.audio
                    .execute(super::audio::DownloadInput {
                        message_id: input.message_id,
                        chat_id: input.chat_id,
                        params: input.params,
                        url: input.url,
                        chat_cfg: input.chat_cfg,
                        link_is_visible: input.link_is_visible,
                        progress_message_id: None,
                        base_text: None,
                    })
                    .await
            }
            MediaType::Photo => {
                self.photo
                    .execute(super::photo::DownloadInput {
                        message_id: input.message_id,
                        chat_id: input.chat_id,
                        params: input.params,
                        url: input.url,
                        chat_cfg: input.chat_cfg,
                        link_is_visible: input.link_is_visible,
                    })
                    .await
            }
        }
    }
}

// Container extensions that imply a video stream. `webm` is excluded: it is also a common
// audio-only container, and real video webm carries a width to classify on.
fn is_video_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp4" | "mkv" | "mov" | "m4v" | "avi" | "flv" | "ts" | "3gp" | "wmv"
    )
}
