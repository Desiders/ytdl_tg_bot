use crate::{
    models::yt_toolkit::{BasicInfo, PlayabilityStatus, VideoInfoKind},
    utils::{get_video_id, GetVideoIdErrorKind},
};

use reqwest::Client;

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
