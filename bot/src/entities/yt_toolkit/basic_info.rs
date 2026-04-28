use serde::Deserialize;
use url::Url;

use crate::entities::ShortMedia;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicInfo {
    pub id: String,
    pub title: Option<String>,
    pub thumbnail: Option<Url>,
}

impl From<BasicInfo> for ShortMedia {
    fn from(BasicInfo { id, title, thumbnail }: BasicInfo) -> Self {
        Self { id, title, thumbnail }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicSearchInfo {
    pub id: String,
    pub title: Option<String>,
    pub thumbnail: Option<Url>,
}

impl From<BasicSearchInfo> for BasicInfo {
    fn from(BasicSearchInfo { id, title, thumbnail }: BasicSearchInfo) -> Self {
        Self { id, title, thumbnail }
    }
}

#[derive(Debug, Deserialize)]
pub struct PlayabilityStatus {
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfoResponse {
    pub basic_info: Option<BasicInfo>,
    pub playability_status: Option<PlayabilityStatus>,
}

#[cfg(test)]
mod tests {
    use super::VideoInfoResponse;

    #[test]
    fn parses_video_info_without_thumbnail() {
        let body = r#"{"basicInfo":{"id":"abc123","title":"Test title"}}"#;
        let response: VideoInfoResponse = serde_json::from_str(body).unwrap();

        assert!(response.basic_info.is_some());
        assert!(response.playability_status.is_none());
        let basic_info = response.basic_info.unwrap();
        assert_eq!(basic_info.id, "abc123");
        assert_eq!(basic_info.title.as_deref(), Some("Test title"));
        assert!(basic_info.thumbnail.is_none());
    }

    #[test]
    fn parses_unplayable_video_info() {
        let body = r#"{"playabilityStatus":{"status":"ERROR","reason":"Video unavailable"}}"#;
        let response: VideoInfoResponse = serde_json::from_str(body).unwrap();

        assert!(response.basic_info.is_none());
        let playability = response.playability_status.unwrap();
        assert_eq!(playability.status, "ERROR");
        assert_eq!(playability.reason.as_deref(), Some("Video unavailable"));
    }
}
