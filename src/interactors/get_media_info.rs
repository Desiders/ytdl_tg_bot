use crate::{
    config::{YtDlpConfig, YtPotProviderConfig, YtToolkitConfig},
    database::TxManager,
    entities::{yt_toolkit::BasicInfo, Cookies, DownloadedMedia, Range, TgAudioInPlaylist, TgVideoInPlaylist, Video, VideosInYT},
    errors::database::ErrorKind,
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
use tracing::{event, instrument, Level};
use url::{Host, Url};

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
    pub tx_manager: TxManager,
}

impl<'a> GetVideoByURLInput<'a> {
    pub const fn new(url: &'a Url, range: &'a Range, tx_manager: TxManager) -> Self {
        Self { url, range, tx_manager }
    }
}

pub enum GetVideoByURLKind {
    SingleCached(String),
    PlaylistCached(Vec<TgVideoInPlaylist>),
    SingleUncached(Video),
    PlaylistUncached(VideosInYT),
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
            mut tx_manager,
        }: GetVideoByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await.map_err(ErrorKind::from)?;

        let dao = tx_manager.downloaded_media_dao().unwrap();
        let mut playlist = dao.get_by_url_or_id(url.as_str(), MediaType::Video).await?;

        if playlist.len() == 1 {
            let video = playlist.remove(0);
            event!(Level::INFO, "Got cached media");
            return Ok(Self::Output::SingleCached(video.file_id));
        }
        if playlist.len() > 1 {
            playlist.sort_by_key(|DownloadedMedia { index_in_playlist, .. }| *index_in_playlist);
            event!(Level::INFO, "Got cached playlist");
            return Ok(Self::Output::PlaylistCached(
                playlist
                    .into_iter()
                    .map(
                        |DownloadedMedia {
                             file_id,
                             index_in_playlist,
                             ..
                         }| TgVideoInPlaylist::new(file_id.into_boxed_str(), index_in_playlist as usize),
                    )
                    .collect(),
            ));
        }

        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        event!(Level::DEBUG, "Getting media info");

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
            event!(Level::INFO, "Got media");
            return Ok(Self::Output::SingleUncached(playlist.remove(0)));
        }
        if playlist.len() > 1 {
            event!(Level::INFO, "Got playlist");
            return Ok(Self::Output::PlaylistUncached(playlist));
        }

        event!(Level::WARN, "Empty playlist");
        Ok(Self::Output::Empty)
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
    pub tx_manager: TxManager,
}

impl<'a> GetAudioByURLInput<'a> {
    pub const fn new(url: &'a Url, range: &'a Range, tx_manager: TxManager) -> Self {
        Self { url, range, tx_manager }
    }
}

pub enum GetAudioByURLKind {
    SingleCached(String),
    PlaylistCached(Vec<TgAudioInPlaylist>),
    SingleUncached(Video),
    PlaylistUncached(VideosInYT),
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
            mut tx_manager,
        }: GetAudioByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await.map_err(ErrorKind::from)?;

        let dao = tx_manager.downloaded_media_dao().unwrap();
        let mut playlist = dao.get_by_url_or_id(url.as_str(), MediaType::Audio).await?;

        if playlist.len() == 1 {
            let video = playlist.remove(0);
            event!(Level::INFO, "Got cached media");
            return Ok(Self::Output::SingleCached(video.file_id));
        }
        if playlist.len() > 1 {
            playlist.sort_by_key(|DownloadedMedia { index_in_playlist, .. }| *index_in_playlist);
            event!(Level::INFO, "Got cached playlist");
            return Ok(Self::Output::PlaylistCached(
                playlist
                    .into_iter()
                    .map(
                        |DownloadedMedia {
                             file_id,
                             index_in_playlist,
                             ..
                         }| TgAudioInPlaylist::new(file_id.into_boxed_str(), index_in_playlist as usize),
                    )
                    .collect(),
            ));
        }

        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        event!(Level::DEBUG, "Getting media info");

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
            event!(Level::INFO, "Got media");
            return Ok(Self::Output::SingleUncached(playlist.remove(0)));
        }
        if playlist.len() > 1 {
            event!(Level::INFO, "Got playlist");
            return Ok(Self::Output::PlaylistUncached(playlist));
        }

        event!(Level::WARN, "Empty playlist");
        Ok(Self::Output::Empty)
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
    Single(Video),
    Playlist(VideosInYT),
    Empty,
}

impl Interactor<GetUncachedVideoByURLInput<'_>> for &GetUncachedVideoByURL {
    type Output = GetUncachedVideoByURLKind;
    type Err = ytdl::Error;

    async fn execute(self, GetUncachedVideoByURLInput { url, range }: GetUncachedVideoByURLInput<'_>) -> Result<Self::Output, Self::Err> {
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        event!(Level::DEBUG, "Getting media info");

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
            event!(Level::INFO, "Got media");
            return Ok(Self::Output::Single(playlist.remove(0)));
        }
        if playlist.len() > 1 {
            event!(Level::INFO, "Got playlist");
            return Ok(Self::Output::Playlist(playlist));
        }

        event!(Level::WARN, "Empty playlist");
        Ok(Self::Output::Empty)
    }
}

pub struct GetMediaInfoById {
    yt_dlp_cfg: Arc<YtDlpConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
}

impl GetMediaInfoById {
    pub const fn new(yt_dlp_cfg: Arc<YtDlpConfig>, yt_pot_provider_cfg: Arc<YtPotProviderConfig>, cookies: Arc<Cookies>) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
        }
    }
}

pub struct GetMediaInfoByIdInput<'a> {
    pub id: &'a str,
    pub host: &'a Host<&'a str>,
    pub range: &'a Range,
}

impl<'a> GetMediaInfoByIdInput<'a> {
    pub const fn new(id: &'a str, host: &'a Host<&'a str>, range: &'a Range) -> Self {
        Self { id, host, range }
    }
}

impl Interactor<GetMediaInfoByIdInput<'_>> for &GetMediaInfoById {
    type Output = VideosInYT;
    type Err = ytdl::Error;

    #[instrument(skip_all)]
    async fn execute(self, GetMediaInfoByIdInput { id, host, range }: GetMediaInfoByIdInput<'_>) -> Result<Self::Output, Self::Err> {
        let cookie = self.cookies.get_path_by_host(host);

        event!(Level::DEBUG, "Getting media info");
        let res = get_media_or_playlist_info(
            self.yt_dlp_cfg.executable_path.as_ref(),
            format!("ytsearch:{id}"),
            self.yt_pot_provider_cfg.url.as_ref(),
            true,
            GET_INFO_TIMEOUT,
            range,
            cookie,
        )
        .await;
        event!(Level::INFO, "Got media info");
        res
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
        event!(Level::DEBUG, "Getting media info");
        let res = get_video_info(&self.client, self.yt_toolkit_cfg.url.as_ref(), url.as_ref()).await?;
        event!(Level::INFO, "Got media info");
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
        event!(Level::DEBUG, "Searching media info");
        let res = search_video(&self.client, self.yt_toolkit_cfg.url.as_ref(), text).await?;
        event!(Level::INFO, "Got media info");
        Ok(res)
    }
}
