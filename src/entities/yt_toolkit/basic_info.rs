use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicInfo {
    pub id: String,
    pub title: String,
    pub thumbnail: Vec<String>,
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicSearchInfo {
    pub id: String,
    pub title: String,
    pub thumbnails: Vec<String>,
}

impl From<BasicSearchInfo> for BasicInfo {
    fn from(BasicSearchInfo { id, title, thumbnails }: BasicSearchInfo) -> Self {
        Self {
            id,
            title,
            thumbnail: thumbnails,
            width: 1,
            height: 1,
        }
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
