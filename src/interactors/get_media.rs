use crate::{
    config::{YtDlpConfig, YtPotProviderConfig, YtToolkitConfig},
    database::TxManager,
    entities::{
        language::Language, yt_toolkit::BasicInfo, Cookies, DownloadedMedia, Media, MediaFormat, MediaInPlaylist, Playlist, Range,
        RawMediaWithFormat, Sections,
    },
    errors::ErrorKind,
    interactors::Interactor,
    services::{
        yt_toolkit::{get_video_info, search_video, GetVideoInfoErrorKind, SearchVideoErrorKind},
        ytdl::{get_media_info, FormatStrategy, GetInfoErrorKind},
    },
    value_objects::MediaType,
};

use reqwest::Client;
use std::{convert::Infallible, sync::Arc};
use tracing::{debug, info, instrument, warn};
use url::Url;

const GET_INFO_TIMEOUT: u64 = 180;

#[derive(Debug, thiserror::Error)]
pub enum GetMediaByURLErrorKind {
    #[error(transparent)]
    GetInfo(#[from] GetInfoErrorKind),
    #[error(transparent)]
    Database(#[from] ErrorKind<Infallible>),
}

pub struct GetMediaByURLInput<'a> {
    pub url: &'a Url,
    pub playlist_range: &'a Range,
    pub cache_search: &'a str,
    pub domain: Option<&'a str>,
    pub audio_language: &'a Language,
    pub sections: Option<&'a Sections>,
    pub tx_manager: &'a mut TxManager,
}

pub struct GetUncachedMediaByURLInput<'a> {
    pub url: &'a Url,
    pub playlist_range: &'a Range,
    pub audio_language: &'a Language,
}

pub enum GetMediaByURLKind {
    SingleCached(String),
    Playlist {
        cached: Vec<MediaInPlaylist>,
        uncached: Vec<(Media, Vec<(MediaFormat, RawMediaWithFormat)>)>,
    },
    Empty,
}

pub struct GetVideoByURL {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub cookies: Arc<Cookies>,
}

impl Interactor<GetMediaByURLInput<'_>> for &GetVideoByURL {
    type Output = GetMediaByURLKind;
    type Err = GetMediaByURLErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        GetMediaByURLInput {
            url,
            playlist_range,
            cache_search,
            domain,
            audio_language,
            sections,
            tx_manager,
        }: GetMediaByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await.map_err(ErrorKind::from)?;

        let dao = tx_manager.downloaded_media_dao().unwrap();
        let is_single_media = playlist_range.is_single_element();
        let (start, end) = match sections {
            Some(sections) => (sections.start, sections.end),
            None => {
                let val = Sections::default();
                (val.start, val.end)
            }
        };

        if is_single_media {
            if let Some(media) = dao
                .get(
                    cache_search,
                    domain,
                    audio_language.language.as_deref(),
                    MediaType::Video,
                    start,
                    end,
                )
                .await?
            {
                info!("Got cached media");
                return Ok(Self::Output::SingleCached(media.file_id));
            }
        }

        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        debug!("Getting media");

        let playlist = get_media_info(
            url.as_str(),
            &FormatStrategy::VideoAndAudio,
            audio_language,
            &self.yt_dlp_cfg.executable_path,
            &self.yt_pot_provider_cfg.url,
            playlist_range,
            !is_single_media,
            GET_INFO_TIMEOUT,
            cookie,
        )
        .await?;
        let playlist_len = playlist.inner.len();

        let mut cached = vec![];
        let mut uncached = vec![];
        for (media, mut formats) in playlist.inner {
            let domain = media.webpage_url.domain();
            if let Some(DownloadedMedia { file_id, .. }) = dao
                .get(&media.id, domain, audio_language.language.as_deref(), MediaType::Video, start, end)
                .await?
            {
                cached.push(MediaInPlaylist {
                    file_id,
                    playlist_index: media.playlist_index,
                });
                continue;
            }
            formats.retain(|(format, _)| {
                if let Some(val) = format.filesize_approx {
                    val <= self.yt_dlp_cfg.max_file_size
                } else {
                    true
                }
            });
            uncached.push((media, formats));
        }
        if playlist_len == 0 {
            warn!("Empty playlist");
            return Ok(Self::Output::Empty);
        }

        info!(playlist_len, cached_len = cached.len(), unchached_len = uncached.len(), "Got media");
        Ok(Self::Output::Playlist { cached, uncached })
    }
}

pub struct GetAudioByURL {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub cookies: Arc<Cookies>,
}

impl Interactor<GetMediaByURLInput<'_>> for &GetAudioByURL {
    type Output = GetMediaByURLKind;
    type Err = GetMediaByURLErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        GetMediaByURLInput {
            url,
            playlist_range,
            cache_search,
            domain,
            audio_language,
            sections,
            tx_manager,
        }: GetMediaByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await.map_err(ErrorKind::from)?;

        let dao = tx_manager.downloaded_media_dao().unwrap();
        let is_single_media = playlist_range.is_single_element();
        let (start, end) = match sections {
            Some(sections) => (sections.start, sections.end),
            None => {
                let val = Sections::default();
                (val.start, val.end)
            }
        };

        if is_single_media {
            if let Some(media) = dao
                .get(
                    cache_search,
                    domain,
                    audio_language.language.as_deref(),
                    MediaType::Audio,
                    start,
                    end,
                )
                .await?
            {
                info!("Got cached media");
                return Ok(Self::Output::SingleCached(media.file_id));
            }
        }

        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());
        let audio_ext = "m4a";

        debug!("Getting media");

        let playlist = get_media_info(
            url.as_str(),
            &FormatStrategy::AudioOnly { audio_ext },
            audio_language,
            &self.yt_dlp_cfg.executable_path,
            &self.yt_pot_provider_cfg.url,
            playlist_range,
            !is_single_media,
            GET_INFO_TIMEOUT,
            cookie,
        )
        .await?;
        let playlist_len = playlist.inner.len();

        let mut cached = vec![];
        let mut uncached = vec![];
        for (media, mut formats) in playlist.inner {
            let domain = media.webpage_url.domain();
            if let Some(DownloadedMedia { file_id, .. }) = dao
                .get(&media.id, domain, audio_language.language.as_deref(), MediaType::Audio, start, end)
                .await?
            {
                cached.push(MediaInPlaylist {
                    file_id,
                    playlist_index: media.playlist_index,
                });
                continue;
            }
            formats.retain(|(format, _)| {
                if let Some(val) = format.filesize_approx {
                    val <= self.yt_dlp_cfg.max_file_size
                } else {
                    true
                }
            });
            uncached.push((media, formats));
        }
        if playlist_len == 0 {
            warn!("Empty playlist");
            return Ok(Self::Output::Empty);
        }

        info!(playlist_len, cached_len = cached.len(), unchached_len = uncached.len(), "Got media");
        Ok(Self::Output::Playlist { cached, uncached })
    }
}

pub struct GetUncachedVideoByURL {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub cookies: Arc<Cookies>,
}

impl Interactor<GetUncachedMediaByURLInput<'_>> for &GetUncachedVideoByURL {
    type Output = Playlist;
    type Err = GetInfoErrorKind;

    async fn execute(
        self,
        GetUncachedMediaByURLInput {
            url,
            playlist_range,
            audio_language,
        }: GetUncachedMediaByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());
        let is_single_media = playlist_range.is_single_element();

        debug!("Getting media");

        let playlist = get_media_info(
            url.as_str(),
            &FormatStrategy::VideoAndAudio,
            audio_language,
            &self.yt_dlp_cfg.executable_path,
            &self.yt_pot_provider_cfg.url,
            playlist_range,
            !is_single_media,
            GET_INFO_TIMEOUT,
            cookie,
        )
        .await?;
        let playlist_len = playlist.inner.len();

        info!(playlist_len, "Got media");
        Ok(playlist)
    }
}

pub struct GetShortMediaByURL {
    pub client: Arc<Client>,
    pub yt_toolkit_cfg: Arc<YtToolkitConfig>,
}

pub struct GetShortMediaByURLInput<'a> {
    pub url: &'a Url,
}

impl Interactor<GetShortMediaByURLInput<'_>> for &GetShortMediaByURL {
    type Output = Vec<BasicInfo>;
    type Err = GetVideoInfoErrorKind;

    #[instrument(skip_all)]
    async fn execute(self, GetShortMediaByURLInput { url }: GetShortMediaByURLInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Getting media");
        let res = get_video_info(&self.client, &self.yt_toolkit_cfg.url, url.as_str()).await?;
        info!("Got media");
        Ok(res)
    }
}

pub struct SearchMediaInfo {
    pub client: Arc<Client>,
    pub yt_toolkit_cfg: Arc<YtToolkitConfig>,
}

pub struct SearchMediaInfoInput<'a> {
    pub text: &'a str,
}

impl Interactor<SearchMediaInfoInput<'_>> for &SearchMediaInfo {
    type Output = Vec<BasicInfo>;
    type Err = SearchVideoErrorKind;

    #[instrument(skip_all)]
    async fn execute(self, SearchMediaInfoInput { text }: SearchMediaInfoInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Searching media");
        let res = search_video(&self.client, &self.yt_toolkit_cfg.url, text).await?;
        info!("Got media");
        Ok(res)
    }
}
