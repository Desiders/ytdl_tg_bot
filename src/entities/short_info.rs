use super::{
    video::{Thumbnail as YtDlpThumbnail, Video},
    yt_toolkit::BasicInfo,
};
use crate::utils::{calculate_aspect_ratio, get_nearest_to_aspect, get_urls_by_aspect};

use serde::Deserialize;
use url::Host;

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
    pub fn thumbnail_urls(&self, service_host: Option<&Host<&str>>) -> Vec<String> {
        let aspect_ratio = calculate_aspect_ratio(self.width, self.height);
        let aspect_kind = get_nearest_to_aspect(aspect_ratio);
        let mut thumbnail_urls = get_urls_by_aspect(service_host, &self.id, aspect_kind);

        for Thumbnail { url } in &self.thumbnails {
            if let Some(url) = url.clone() {
                thumbnail_urls.push(url);
            }
        }
        thumbnail_urls
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
