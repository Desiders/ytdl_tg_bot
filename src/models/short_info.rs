use super::{
    video::{Thumbnail as YtDlpThumbnail, Video},
    yt_toolkit::BasicInfo,
};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Thumbnail {
    pub url: Option<String>,
}

impl From<String> for Thumbnail {
    fn from(url: String) -> Self {
        Self { url: Some(url) }
    }
}

impl From<YtDlpThumbnail> for Thumbnail {
    fn from(YtDlpThumbnail { url }: YtDlpThumbnail) -> Self {
        Self { url }
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

impl From<Video> for ShortInfo {
    fn from(
        Video {
            title,
            thumbnail,
            thumbnails,
            ..
        }: Video,
    ) -> Self {
        Self {
            title,
            thumbnails: {
                let mut all_thumbnails = thumbnail.map(|url| vec![Thumbnail { url: Some(url) }]).unwrap_or_default();
                all_thumbnails.extend(thumbnails.unwrap_or_default().into_iter().map(Into::into));
                all_thumbnails
            },
        }
    }
}
