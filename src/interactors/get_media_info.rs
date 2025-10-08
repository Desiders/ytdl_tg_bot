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
use url::Url;

const GET_INFO_TIMEOUT: u64 = 180;

pub struct GetMediaInfo {
    yt_dlp_cfg: Arc<YtDlpConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
}

impl GetMediaInfo {
    pub const fn new(yt_dlp_cfg: Arc<YtDlpConfig>, yt_pot_provider_cfg: Arc<YtPotProviderConfig>, cookies: Arc<Cookies>) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
        }
    }
}

pub struct GetMedaInfoInput<'a> {
    pub url: &'a Url,
    pub range: &'a Range,
}

impl<'a> GetMedaInfoInput<'a> {
    pub const fn new(url: &'a Url, range: &'a Range) -> Self {
        Self { url, range }
    }
}

impl Interactor for GetMediaInfo {
    type Input<'a> = GetMedaInfoInput<'a>;
    type Output = VideosInYT;
    type Err = ytdl::Error;

    #[instrument(target = "get_info", skip_all)]
    async fn execute(&mut self, GetMedaInfoInput { url, range }: Self::Input<'_>) -> Result<Self::Output, Self::Err> {
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

pub struct GetShortMediaInfo {
    client: Arc<Client>,
    yt_toolkit_cfg: Arc<YtToolkitConfig>,
}

impl GetShortMediaInfo {
    pub const fn new(client: Arc<Client>, yt_toolkit_cfg: Arc<YtToolkitConfig>) -> Self {
        Self { client, yt_toolkit_cfg }
    }
}

pub struct GetShortMediaInfoInput<'a> {
    pub url: &'a Url,
}

impl<'a> GetShortMediaInfoInput<'a> {
    pub const fn new(url: &'a Url) -> Self {
        Self { url }
    }
}

impl Interactor for GetShortMediaInfo {
    type Input<'a> = GetShortMediaInfoInput<'a>;
    type Output = Vec<BasicInfo>;
    type Err = GetVideoInfoErrorKind;

    #[instrument(target = "get_info", skip_all)]
    async fn execute(&mut self, GetShortMediaInfoInput { url }: Self::Input<'_>) -> Result<Self::Output, Self::Err> {
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

impl Interactor for SearchMediaInfo {
    type Input<'a> = SearchMediaInfoInput<'a>;
    type Output = Vec<BasicInfo>;
    type Err = SearchVideoErrorKind;

    #[instrument(target = "get_info", skip_all)]
    async fn execute(&mut self, SearchMediaInfoInput { text }: Self::Input<'_>) -> Result<Self::Output, Self::Err> {
        event!(Level::DEBUG, "Searching media info");
        let res = search_video(&self.client, self.yt_toolkit_cfg.url.as_ref(), text).await?;
        event!(Level::INFO, "Got media info");
        Ok(res)
    }
}
