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
    pub id: String,
    pub title: Option<String>,
    pub thumbnails: Vec<Thumbnail>,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

impl ShortInfo {
    pub fn thumbnail(&self) -> Option<&str> {
        let preferred_order = ["maxresdefault", "hq720", "sddefault", "hqdefault", "mqdefault", "default"];

        self.thumbnails
            .iter()
            .filter_map(|thumbnail| thumbnail.url.as_deref())
            .max_by_key(|url| preferred_order.iter().position(|&name| url.contains(name)))
    }
}

impl From<BasicInfo> for ShortInfo {
    fn from(
        BasicInfo {
            id,
            title,
            thumbnail,
            width,
            height,
        }: BasicInfo,
    ) -> Self {
        ShortInfo {
            id,
            title: Some(title),
            thumbnails: thumbnail.into_iter().map(Into::into).collect(),
            width: Some(width),
            height: Some(height),
        }
    }
}

impl From<Video> for ShortInfo {
    fn from(
        Video {
            id,
            title,
            thumbnail,
            thumbnails,
            width,
            height,
            ..
        }: Video,
    ) -> Self {
        Self {
            id,
            title,
            thumbnails: {
                let mut all_thumbnails = thumbnail.map(|url| vec![Thumbnail { url: Some(url) }]).unwrap_or_default();
                all_thumbnails.extend(thumbnails.unwrap_or_default().into_iter().map(Into::into));
                all_thumbnails
            },
            width,
            height,
        }
    }
}
