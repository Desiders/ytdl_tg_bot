use crate::{
    models::yt_toolkit::{BasicInfo, BasicSearchInfo, PlayabilityStatus, VideoInfoKind},
    utils::{get_video_id, GetVideoIdErrorKind},
};

use reqwest::Client;
use url::Url;

#[derive(thiserror::Error, Debug)]
pub enum GetVideoInfoErrorKind {
    #[error(transparent)]
    GetVideoId(#[from] GetVideoIdErrorKind),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("Video unplayable. Status: {} and reason: {}", ._0.status, ._0.reason.as_deref().unwrap_or("unknown"))]
    Unplayable(PlayabilityStatus),
}

pub async fn get_video_info(client: Client, api_url: &str, url: &str) -> Result<Vec<BasicInfo>, GetVideoInfoErrorKind> {
    let id = get_video_id(url)?;

    match serde_json::from_str::<VideoInfoKind>(
        &client
            .get(&format!("{api_url}/video"))
            .query(&[("id", &id)])
            .send()
            .await?
            .text()
            .await?,
    )? {
        VideoInfoKind::Playable { basic_info } => Ok(vec![basic_info]),
        VideoInfoKind::Unplayable { playability_status } => Err(GetVideoInfoErrorKind::Unplayable(playability_status)),
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SearchVideoErrorKind {
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub async fn search_video(client: Client, api_url: &str, text: &str) -> Result<Vec<BasicInfo>, SearchVideoErrorKind> {
    let basic_search_info = serde_json::from_str::<Vec<BasicSearchInfo>>(
        &client
            .get(&format!("{api_url}/search"))
            .query(&[("q", &text)])
            .send()
            .await?
            .text()
            .await?,
    )?;

    Ok(basic_search_info.into_iter().map(Into::into).collect())
}

pub fn get_thumbnail_url(api_url: &str, id: &str, width: i64, height: i64) -> Result<String, GetVideoIdErrorKind> {
    Ok(Url::parse_with_params(
        &format!("{api_url}/thumbnail"),
        &[("id", id), ("width", &width.to_string()), ("height", &height.to_string())],
    )
    .unwrap()
    .into())
}
