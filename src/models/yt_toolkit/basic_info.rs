use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicInfo {
    pub title: String,
    pub thumbnail: Vec<String>,
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
