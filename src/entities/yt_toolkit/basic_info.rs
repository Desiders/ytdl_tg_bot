use serde::Deserialize;
use url::Url;

use crate::entities::ShortMedia;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicInfo {
    pub id: String,
    pub title: String,
    pub thumbnail: Url,
}

impl From<BasicInfo> for ShortMedia {
    fn from(BasicInfo { id, title, thumbnail }: BasicInfo) -> Self {
        Self {
            id,
            title: Some(title),
            thumbnail: Some(thumbnail),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicSearchInfo {
    pub id: String,
    pub title: String,
    pub thumbnail: Url,
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
#[serde(untagged)]
pub enum VideoInfoKind {
    #[serde(rename_all = "camelCase")]
    Playable { basic_info: BasicInfo },
    #[serde(rename_all = "camelCase")]
    Unplayable { playability_status: PlayabilityStatus },
}
