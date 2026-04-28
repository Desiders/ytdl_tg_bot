use crate::{
    entities::yt_toolkit::{BasicInfo, BasicSearchInfo, PlayabilityStatus, VideoInfoResponse},
    utils::{get_video_id, GetVideoIdErrorKind},
};

use reqwest::Client;
use tracing::{instrument, warn};

fn shorten_response_body(body: &str) -> String {
    const MAX_LEN: usize = 512;
    if body.len() <= MAX_LEN {
        body.to_owned()
    } else {
        format!("{}...", &body[..MAX_LEN])
    }
}

#[derive(thiserror::Error, Debug)]
pub enum GetVideoInfoErrorKind {
    #[error(transparent)]
    GetVideoId(#[from] GetVideoIdErrorKind),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("YT Toolkit returned HTTP {status}: {body}")]
    HttpStatus { status: reqwest::StatusCode, body: String },
    #[error("Video unplayable. Status: {} and reason: {}", ._0.status, ._0.reason.as_deref().unwrap_or("unknown"))]
    Unplayable(PlayabilityStatus),
    #[error("Invalid video info response")]
    InvalidResponse,
}

#[instrument(skip_all)]
pub async fn get_video_info(client: &Client, api_url: &str, url: &str) -> Result<Vec<BasicInfo>, GetVideoInfoErrorKind> {
    let id = get_video_id(url)?;
    let response = client
        .get(format!("{api_url}/video"))
        .query(&[("id", &id)])
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        warn!(%status, video_id = %id, response_body = %shorten_response_body(&body), "YT Toolkit /video returned non-success status");
        return Err(GetVideoInfoErrorKind::HttpStatus {
            status,
            body: shorten_response_body(&body),
        });
    }

    let response = match serde_json::from_str::<VideoInfoResponse>(&body) {
        Ok(response) => response,
        Err(err) => {
            warn!(
                video_id = %id,
                response_body = %shorten_response_body(&body),
                parse_error = %err,
                "Failed to parse YT Toolkit /video response"
            );
            return Err(err.into());
        }
    };

    if let Some(basic_info) = response.basic_info {
        Ok(vec![basic_info])
    } else if let Some(playability_status) = response.playability_status {
        Err(GetVideoInfoErrorKind::Unplayable(playability_status))
    } else {
        warn!(
            video_id = %id,
            response_body = %shorten_response_body(&body),
            "YT Toolkit /video response had no basicInfo or playabilityStatus"
        );
        Err(GetVideoInfoErrorKind::InvalidResponse)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SearchVideoErrorKind {
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("YT Toolkit returned HTTP {status}: {body}")]
    HttpStatus { status: reqwest::StatusCode, body: String },
}

#[instrument(skip_all)]
pub async fn search_video(client: &Client, api_url: &str, text: &str) -> Result<Vec<BasicInfo>, SearchVideoErrorKind> {
    let response = client
        .get(format!("{api_url}/search"))
        .query(&[("q", &text)])
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        warn!(%status, query = text, response_body = %shorten_response_body(&body), "YT Toolkit /search returned non-success status");
        return Err(SearchVideoErrorKind::HttpStatus {
            status,
            body: shorten_response_body(&body),
        });
    }

    let basic_search_info = match serde_json::from_str::<Vec<BasicSearchInfo>>(&body) {
        Ok(info) => info,
        Err(err) => {
            warn!(
                query = text,
                response_body = %shorten_response_body(&body),
                parse_error = %err,
                "Failed to parse YT Toolkit /search response"
            );
            return Err(err.into());
        }
    };
    Ok(basic_search_info.into_iter().map(Into::into).collect())
}
