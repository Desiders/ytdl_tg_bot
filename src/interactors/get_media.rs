use crate::{
    config::{YtDlpConfig, YtPotProviderConfig, YtToolkitConfig},
    database::TxManager,
    entities::{yt_toolkit::BasicInfo, Cookies, DownloadedMedia, Range, TgAudioInPlaylist, TgVideoInPlaylist, Video, VideosInYT},
    errors::ErrorKind,
    interactors::Interactor,
    services::{
        get_media_or_playlist_info,
        yt_toolkit::{get_video_info, search_video, GetVideoInfoErrorKind, SearchVideoErrorKind},
        ytdl,
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
    Ytdl(#[from] ytdl::Error),
    #[error(transparent)]
    Database(#[from] ErrorKind<Infallible>),
}

pub struct GetVideoByURL {
    yt_dlp_cfg: Arc<YtDlpConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
}

impl GetVideoByURL {
    pub const fn new(yt_dlp_cfg: Arc<YtDlpConfig>, yt_pot_provider_cfg: Arc<YtPotProviderConfig>, cookies: Arc<Cookies>) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
        }
    }
}

pub struct GetVideoByURLInput<'a> {
    pub url: &'a Url,
    pub range: &'a Range,
    pub id_or_url: &'a str,
    pub domain: Option<&'a str>,
    pub tx_manager: &'a mut TxManager,
}

impl<'a> GetVideoByURLInput<'a> {
    pub const fn new(url: &'a Url, range: &'a Range, id_or_url: &'a str, domain: Option<&'a str>, tx_manager: &'a mut TxManager) -> Self {
        Self {
            url,
            range,
            id_or_url,
            domain,
            tx_manager,
        }
    }
}

pub enum GetVideoByURLKind {
    SingleCached(String),
    Playlist((Vec<TgVideoInPlaylist>, Vec<Video>)),
    Empty,
}

impl Interactor<GetVideoByURLInput<'_>> for &GetVideoByURL {
    type Output = GetVideoByURLKind;
    type Err = GetMediaByURLErrorKind;

    async fn execute(
        self,
        GetVideoByURLInput {
            url,
            range,
            id_or_url,
            domain,
            tx_manager,
        }: GetVideoByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await.map_err(ErrorKind::from)?;

        let dao = tx_manager.downloaded_media_dao().unwrap();

        if range.is_single_element() {
            let normalized_domain = domain.map(|domain| domain.trim_start_matches("www."));

            if let Some(media) = dao
                .get_by_id_or_url_and_domain(id_or_url, normalized_domain, MediaType::Video)
                .await?
            {
                info!("Got cached media");
                return Ok(Self::Output::SingleCached(media.file_id));
            }
        }

        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        debug!("Getting media info");

        let playlist = get_media_or_playlist_info(
            self.yt_dlp_cfg.executable_path.as_ref(),
            url,
            self.yt_pot_provider_cfg.url.as_ref(),
            true,
            GET_INFO_TIMEOUT,
            range,
            cookie,
        )
        .await?;
        let playlist_len = playlist.len();

        let mut cached = vec![];
        let mut uncached = vec![];
        for (index, media) in playlist.into_iter().enumerate() {
            let domain = media.domain();
            let normalized_domain = domain.as_deref().map(|domain| domain.trim_start_matches("www"));

            if let Some(DownloadedMedia { file_id, .. }) = dao
                .get_by_id_or_url_and_domain(&media.id, normalized_domain, MediaType::Video)
                .await?
            {
                cached.push(TgVideoInPlaylist {
                    file_id: file_id.into(),
                    index,
                });
                continue;
            }
            uncached.push(media);
        }
        if playlist_len == 0 {
            warn!("Empty playlist");
            return Ok(Self::Output::Empty);
        }

        info!(
            playlist_len,
            cached_len = cached.len(),
            unchached_len = uncached.len(),
            "Got media info"
        );
        Ok(Self::Output::Playlist((cached, uncached)))
    }
}

pub struct GetAudioByURL {
    yt_dlp_cfg: Arc<YtDlpConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
}

impl GetAudioByURL {
    pub const fn new(yt_dlp_cfg: Arc<YtDlpConfig>, yt_pot_provider_cfg: Arc<YtPotProviderConfig>, cookies: Arc<Cookies>) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
        }
    }
}

pub struct GetAudioByURLInput<'a> {
    pub url: &'a Url,
    pub range: &'a Range,
    pub id: &'a str,
    pub domain: Option<&'a str>,
    pub tx_manager: &'a mut TxManager,
}

impl<'a> GetAudioByURLInput<'a> {
    pub const fn new(url: &'a Url, range: &'a Range, id: &'a str, domain: Option<&'a str>, tx_manager: &'a mut TxManager) -> Self {
        Self {
            url,
            range,
            id,
            domain,
            tx_manager,
        }
    }
}

pub enum GetAudioByURLKind {
    SingleCached(String),
    Playlist((Vec<TgAudioInPlaylist>, Vec<Video>)),
    Empty,
}

impl Interactor<GetAudioByURLInput<'_>> for &GetAudioByURL {
    type Output = GetAudioByURLKind;
    type Err = GetMediaByURLErrorKind;

    async fn execute(
        self,
        GetAudioByURLInput {
            url,
            range,
            id,
            domain,
            tx_manager,
        }: GetAudioByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await.map_err(ErrorKind::from)?;

        let dao = tx_manager.downloaded_media_dao().unwrap();

        if range.is_single_element() {
            let normalized_domain = domain.map(|domain| domain.trim_start_matches("www."));

            if let Some(media) = dao.get_by_id_or_url_and_domain(id, normalized_domain, MediaType::Audio).await? {
                info!("Got cached media");
                return Ok(Self::Output::SingleCached(media.file_id));
            }
        }

        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        debug!("Getting media info");

        let playlist = get_media_or_playlist_info(
            self.yt_dlp_cfg.executable_path.as_ref(),
            url,
            self.yt_pot_provider_cfg.url.as_ref(),
            true,
            GET_INFO_TIMEOUT,
            range,
            cookie,
        )
        .await?;
        let playlist_len = playlist.len();

        let mut cached = vec![];
        let mut uncached = vec![];
        for (index, media) in playlist.into_iter().enumerate() {
            let domain = media.domain();
            let normalized_domain = domain.as_deref().map(|domain| domain.trim_start_matches("www."));

            if let Some(DownloadedMedia { file_id, .. }) = dao
                .get_by_id_or_url_and_domain(&media.id, normalized_domain, MediaType::Audio)
                .await?
            {
                cached.push(TgAudioInPlaylist {
                    file_id: file_id.into(),
                    index,
                });
                continue;
            }
            uncached.push(media);
        }
        if playlist_len == 0 {
            warn!("Empty playlist");
            return Ok(Self::Output::Empty);
        }

        info!(
            playlist_len,
            cached_len = cached.len(),
            unchached_len = uncached.len(),
            "Got media info"
        );
        Ok(Self::Output::Playlist((cached, uncached)))
    }
}

pub struct GetUncachedVideoByURL {
    yt_dlp_cfg: Arc<YtDlpConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
}

impl GetUncachedVideoByURL {
    pub const fn new(yt_dlp_cfg: Arc<YtDlpConfig>, yt_pot_provider_cfg: Arc<YtPotProviderConfig>, cookies: Arc<Cookies>) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
        }
    }
}

pub struct GetUncachedVideoByURLInput<'a> {
    pub url: &'a Url,
    pub range: &'a Range,
}

impl<'a> GetUncachedVideoByURLInput<'a> {
    pub const fn new(url: &'a Url, range: &'a Range) -> Self {
        Self { url, range }
    }
}

pub enum GetUncachedVideoByURLKind {
    Single(Box<Video>),
    Playlist(VideosInYT),
    Empty,
}

impl Interactor<GetUncachedVideoByURLInput<'_>> for &GetUncachedVideoByURL {
    type Output = GetUncachedVideoByURLKind;
    type Err = ytdl::Error;

    async fn execute(self, GetUncachedVideoByURLInput { url, range }: GetUncachedVideoByURLInput<'_>) -> Result<Self::Output, Self::Err> {
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        debug!("Getting media info");

        let mut playlist = get_media_or_playlist_info(
            self.yt_dlp_cfg.executable_path.as_ref(),
            url,
            self.yt_pot_provider_cfg.url.as_ref(),
            true,
            GET_INFO_TIMEOUT,
            range,
            cookie,
        )
        .await?;

        if playlist.len() == 1 {
            info!("Got media");
            return Ok(Self::Output::Single(Box::new(playlist.remove(0))));
        }
        if playlist.len() > 1 {
            info!("Got playlist");
            return Ok(Self::Output::Playlist(playlist));
        }

        warn!("Empty playlist");
        Ok(Self::Output::Empty)
    }
}

pub struct GetShortMediaByURLInfo {
    client: Arc<Client>,
    yt_toolkit_cfg: Arc<YtToolkitConfig>,
}

impl GetShortMediaByURLInfo {
    pub const fn new(client: Arc<Client>, yt_toolkit_cfg: Arc<YtToolkitConfig>) -> Self {
        Self { client, yt_toolkit_cfg }
    }
}

pub struct GetShortMediaInfoByURLInput<'a> {
    pub url: &'a Url,
}

impl<'a> GetShortMediaInfoByURLInput<'a> {
    pub const fn new(url: &'a Url) -> Self {
        Self { url }
    }
}

impl Interactor<GetShortMediaInfoByURLInput<'_>> for &GetShortMediaByURLInfo {
    type Output = Vec<BasicInfo>;
    type Err = GetVideoInfoErrorKind;

    #[instrument(skip_all)]
    async fn execute(self, GetShortMediaInfoByURLInput { url }: GetShortMediaInfoByURLInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Getting media info");
        let res = get_video_info(&self.client, self.yt_toolkit_cfg.url.as_ref(), url.as_ref()).await?;
        info!("Got media info");
        Ok(res)
    }
}

pub struct SearchMediaInfo {
    client: Arc<Client>,
    yt_toolkit_cfg: Arc<YtToolkitConfig>,
}

impl SearchMediaInfo {
    pub const fn new(client: Arc<Client>, yt_toolkit_cfg: Arc<YtToolkitConfig>) -> Self {
        Self { client, yt_toolkit_cfg }
    }
}

pub struct SearchMediaInfoInput<'a> {
    pub text: &'a str,
}

impl<'a> SearchMediaInfoInput<'a> {
    pub const fn new(text: &'a str) -> Self {
        Self { text }
    }
}

impl Interactor<SearchMediaInfoInput<'_>> for &SearchMediaInfo {
    type Output = Vec<BasicInfo>;
    type Err = SearchVideoErrorKind;

    #[instrument(skip_all)]
    async fn execute(self, SearchMediaInfoInput { text }: SearchMediaInfoInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Searching media info");
        let res = search_video(&self.client, self.yt_toolkit_cfg.url.as_ref(), text).await?;
        info!("Got media info");
        Ok(res)
    }
}
