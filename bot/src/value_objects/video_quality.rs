use proto::downloader::VideoQuality as ProtoVideoQuality;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoQuality {
    pub width: i64,
    pub height: i64,
    pub fps: Option<f64>,
}

impl VideoQuality {
    pub fn new(width: Option<i64>, height: Option<i64>, fps: Option<f64>) -> Option<Self> {
        let (width, height) = (width?, height?);
        (width > 0 && height > 0).then_some(Self {
            width,
            height,
            fps: fps.filter(|fps| fps.is_finite() && *fps > 0.0),
        })
    }

    pub fn cache_key(&self) -> String {
        let fps = self.fps.map_or_else(|| "unknown".to_owned(), |fps| fps.to_string());
        format!("{}x{}@{fps}", self.width, self.height)
    }
}

impl From<&VideoQuality> for ProtoVideoQuality {
    fn from(quality: &VideoQuality) -> Self {
        Self {
            width: quality.width,
            height: quality.height,
            fps: quality.fps,
        }
    }
}
