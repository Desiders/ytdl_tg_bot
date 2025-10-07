use super::{combined_format, format};
use crate::{
    entities::PreferredLanguages,
    errors::FormatNotFound,
    utils::{calculate_aspect_ratio, get_nearest_to_aspect, get_url_by_aspect},
};

use serde::Deserialize;
use std::{
    borrow::Cow,
    ops::{Deref, DerefMut},
    path::PathBuf,
    vec,
};
use tempfile::TempDir;
use url::Host;

#[derive(Debug, Clone, Deserialize)]
pub struct Thumbnail {
    pub url: Option<String>,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub thumbnail: Option<String>,
    pub thumbnails: Option<Vec<Thumbnail>>,
    pub original_url: String,
    pub duration: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,

    #[serde(flatten)]
    format: Option<format::Any>,
    #[serde(default)]
    formats: Vec<format::Any>,
}

impl Video {
    pub fn get_combined_formats(&self) -> combined_format::Formats<'_> {
        let mut format_kinds = vec![];

        for format in &self.formats {
            let Ok(format) = format.kind(self.duration) else {
                continue;
            };

            format_kinds.push(format);
        }
        if let Some(format) = &self.format {
            if let Ok(format) = format.kind(self.duration) {
                format_kinds.push(format);
            }
        }

        combined_format::Formats::from(format_kinds)
    }

    pub fn get_audio_formats(&self) -> format::Audios<'_> {
        let mut formats = vec![];

        for format in &self.formats {
            let Ok(format) = format.kind(self.duration) else {
                continue;
            };

            if let format::Kind::Audio(format) = format {
                formats.push(format);
            }
        }
        if let Some(format) = &self.format {
            if let Ok(format::Kind::Audio(format)) = format.kind(self.duration) {
                formats.push(format);
            }
        }

        format::Audios::from(formats)
    }

    fn thumbnail_urls(&self) -> Vec<&str> {
        let mut thumbnail_urls = vec![];
        if let Some(url) = &self.thumbnail {
            thumbnail_urls.push(url.as_ref());
        }
        for Thumbnail { url } in self.thumbnails.as_deref().unwrap_or_default() {
            if let Some(url) = url.as_deref() {
                thumbnail_urls.push(url.as_ref());
            }
        }
        thumbnail_urls
    }

    pub fn thumbnail_url<'a>(&'a self, service_host: Option<&Host<&str>>) -> Option<Cow<'a, str>> {
        let aspect_ratio = calculate_aspect_ratio(self.width, self.height);
        let aspect_kind = get_nearest_to_aspect(aspect_ratio);
        let thumbnail_urls = self.thumbnail_urls();

        match get_url_by_aspect(service_host, &self.id, &thumbnail_urls, aspect_kind) {
            Some(thumbnail_url) => Some(thumbnail_url),
            None => {
                let preferred_order = ["maxresdefault", "hq720", "sddefault", "hqdefault", "mqdefault", "default"];

                thumbnail_urls
                    .into_iter()
                    .map(Cow::Borrowed)
                    .min_by_key(|url| preferred_order.iter().position(|&name| url.contains(name)))
            }
        }
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct VideosInYT(Vec<Video>);

impl VideosInYT {
    pub fn new(videos: impl Into<Vec<Video>>) -> Self {
        Self(videos.into())
    }
}

impl IntoIterator for VideosInYT {
    type Item = Video;
    type IntoIter = vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl Extend<Video> for VideosInYT {
    fn extend<T: IntoIterator<Item = Video>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl Deref for VideosInYT {
    type Target = Vec<Video>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for VideosInYT {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub struct VideoAndFormat<'a> {
    pub video: &'a Video,
    pub format: combined_format::Format<'a>,
}

impl<'a> VideoAndFormat<'a> {
    pub fn new_with_select_format(
        video: &'a Video,
        max_file_size: u32,
        PreferredLanguages { languages }: &PreferredLanguages,
    ) -> Result<Self, FormatNotFound> {
        let mut formats = video.get_combined_formats();
        formats.sort(max_file_size, languages);

        let Some(format) = formats.first().cloned() else {
            return Err(FormatNotFound);
        };

        Ok(Self { video, format })
    }
}

#[derive(Debug)]
pub struct TgVideoInPlaylist {
    pub file_id: Box<str>,
    pub index: usize,
}

impl TgVideoInPlaylist {
    pub fn new(file_id: impl Into<Box<str>>, index: usize) -> Self {
        Self {
            file_id: file_id.into(),
            index,
        }
    }
}

#[derive(Debug)]
pub struct TgVideo {
    pub file_id: Box<str>,
}

impl TgVideo {
    pub fn new(file_id: impl Into<Box<str>>) -> Self {
        Self { file_id: file_id.into() }
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug)]
pub struct VideoInFS {
    pub path: PathBuf,
    pub thumbnail_path: Option<PathBuf>,
    pub temp_dir: TempDir,
}

impl VideoInFS {
    pub fn new(path: impl Into<PathBuf>, thumbnail_path: Option<PathBuf>, temp_dir: TempDir) -> Self {
        Self {
            path: path.into(),
            thumbnail_path,
            temp_dir,
        }
    }
}
