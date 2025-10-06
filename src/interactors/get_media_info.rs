use crate::{
    config::{YtDlpConfig, YtPotProviderConfig},
    entities::{Cookies, Range, VideosInYT},
    interactors::Interactor,
    services::{get_media_or_playlist_info, ytdl},
};

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
    async fn execute<'a>(&mut self, GetMedaInfoInput { url, range }: Self::Input<'a>) -> Result<Self::Output, Self::Err> {
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
        event!(Level::DEBUG, "Got media info");
        res
    }
}
