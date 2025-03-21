use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct StreamsInfo {
    pub streams: Vec<StreamInfo>,
}

#[derive(Debug, Deserialize)]
pub struct StreamInfo {
    pub width: Option<f64>,
    pub height: Option<f64>,
}
