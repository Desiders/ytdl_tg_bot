use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Thumbnail {
    pub url: String,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicInfo {
    // pub id: String,
    pub title: String,
    // pub author: String,
    // pub is_live: bool,
    // pub duration: i64,
    // pub category: String,
    // pub keywords: Vec<String>,
    pub thumbnail: Vec<Thumbnail>,
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
