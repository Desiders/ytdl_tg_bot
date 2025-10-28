use crate::{
    config::{YtDlpConfig, YtPotProviderConfig, YtToolkitConfig},
    entities::{yt_toolkit::BasicInfo, Cookies, Range, VideosInYT},
    interactors::Interactor,
    services::{
        get_media_or_playlist_info,
        yt_toolkit::{get_video_info, search_video, GetVideoInfoErrorKind, SearchVideoErrorKind},
        ytdl,
    },
};

use reqwest::Client;
use std::sync::Arc;
use tracing::{event, instrument, Level};
use url::{Host, Url};

const GET_INFO_TIMEOUT: u64 = 180;

pub struct GetMediaInfoByURL {
    yt_dlp_cfg: Arc<YtDlpConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
}

impl GetMediaInfoByURL {
    pub const fn new(yt_dlp_cfg: Arc<YtDlpConfig>, yt_pot_provider_cfg: Arc<YtPotProviderConfig>, cookies: Arc<Cookies>) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
        }
    }
}

pub struct GetMedaInfoByURLInput<'a> {
    pub url: &'a Url,
    pub range: &'a Range,
}

impl<'a> GetMedaInfoByURLInput<'a> {
    pub const fn new(url: &'a Url, range: &'a Range) -> Self {
        Self { url, range }
    }
}

impl Interactor<GetMedaInfoByURLInput<'_>> for &GetMediaInfoByURL {
    type Output = VideosInYT;
    type Err = ytdl::Error;

    #[instrument(skip_all)]
    async fn execute(self, GetMedaInfoByURLInput { url, range }: GetMedaInfoByURLInput<'_>) -> Result<Self::Output, Self::Err> {
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        event!(Level::DEBUG, "Getting media info");
        let res = get_media_or_playlist_info(
            self.yt_dlp_cfg.executable_path.as_ref(),
            url,
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
