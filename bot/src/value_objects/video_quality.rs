use proto::downloader::VideoQuality as ProtoVideoQuality;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoQuality {
    pub width: i64,
    pub height: i64,
    pub fps: Option<f64>,
    #[serde(default)]
    pub format_id: Option<String>,
    #[serde(default)]
    pub ext: Option<String>,
    #[serde(default)]
    pub vcodec: Option<String>,
    #[serde(default)]
    pub acodec: Option<String>,
}

impl VideoQuality {
    pub fn new(width: Option<i64>, height: Option<i64>, fps: Option<f64>) -> Option<Self> {
        let (width, height) = (width?, height?);
        (width > 0 && height > 0).then_some(Self {
            width,
            height,
            fps: fps.filter(|fps| fps.is_finite() && *fps > 0.0),
            format_id: None,
            ext: None,
            vcodec: None,
            acodec: None,
        })
    }

    pub fn matches(&self, actual: &Self) -> bool {
        self.width == actual.width
            && self.height == actual.height
            && self.fps == actual.fps
            && self.format_id.as_ref().is_none_or(|id| actual.format_id.as_ref() == Some(id))
            && self.ext.as_ref().is_none_or(|ext| actual.ext.as_ref() == Some(ext))
            && self.vcodec.as_ref().is_none_or(|codec| actual.vcodec.as_ref() == Some(codec))
            && self.acodec.as_ref().is_none_or(|codec| actual.acodec.as_ref() == Some(codec))
    }

    pub fn cache_key(&self) -> String {
        let fps = self.fps.map_or_else(|| "unknown".to_owned(), |fps| fps.to_string());
        let resolution = format!("{}x{}@{fps}", self.width, self.height);
        if let Some(id) = &self.format_id {
            format!(
                "{resolution}:{}:{id}:{}:{}:{}",
                id.len(),
                self.ext.as_deref().unwrap_or_default(),
                self.vcodec.as_deref().unwrap_or_default(),
                self.acodec.as_deref().unwrap_or_default()
            )
        } else {
            resolution
        }
    }
}

impl From<&VideoQuality> for ProtoVideoQuality {
    fn from(quality: &VideoQuality) -> Self {
        Self {
            width: quality.width,
            height: quality.height,
            fps: quality.fps,
            format_id: quality.format_id.clone(),
            ext: quality.ext.clone(),
            vcodec: quality.vcodec.clone(),
            acodec: quality.acodec.clone(),
        }
    }
}
