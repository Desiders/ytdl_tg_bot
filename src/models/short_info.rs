use super::{
    video::{Thumbnail as YtDlpThumbnail, VideoInYT},
    yt_toolkit::{BasicInfo, Thumbnail as YtToolkitThumbnail},
};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Thumbnail {
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub url: Option<String>,
}

impl From<YtToolkitThumbnail> for Thumbnail {
    fn from(YtToolkitThumbnail { url, width, height }: YtToolkitThumbnail) -> Self {
        Self {
            width: Some(width),
            height: Some(height),
            url: Some(url),
        }
    }
}

impl From<YtDlpThumbnail> for Thumbnail {
    fn from(YtDlpThumbnail { width, height, url }: YtDlpThumbnail) -> Self {
        Self { width, height, url }
    }
}

#[derive(Debug, Deserialize)]
pub struct ShortInfo {
    pub title: Option<String>,
    pub thumbnails: Vec<Thumbnail>,
}

impl From<BasicInfo> for ShortInfo {
    fn from(BasicInfo { title, thumbnail }: BasicInfo) -> Self {
        ShortInfo {
            title: Some(title),
            thumbnails: thumbnail.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<VideoInYT> for ShortInfo {
    fn from(
        VideoInYT {
            title,
            thumbnail,
            thumbnails,
            ..
        }: VideoInYT,
    ) -> Self {
        Self {
            title,
            thumbnails: {
                let mut all_thumbnails = thumbnail
                    .map(|url| {
                        vec![Thumbnail {
                            width: None,
                            height: None,
                            url: Some(url),
                        }]
                    })
                    .unwrap_or_default();
                all_thumbnails.extend(thumbnails.unwrap_or_default().into_iter().map(Into::into));
                all_thumbnails
            },
        }
    }
}
