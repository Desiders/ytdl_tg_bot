use super::{combined_format, format};
use crate::{
    entities::PreferredLanguages,
    errors::FormatNotFound,
    utils::{calculate_aspect_ratio, get_nearest_to_aspect, get_urls_by_aspect},
};

use serde::Deserialize;
use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
    vec,
};
use tempfile::TempDir;
use url::{Host, Url};

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
    pub webpage_url_domain: Option<String>,
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

    pub fn thumbnail_urls<'a>(&'a self, service_host: Option<&Host<&str>>) -> Vec<String> {
        let aspect_ratio = calculate_aspect_ratio(self.width, self.height);
        let aspect_kind = get_nearest_to_aspect(aspect_ratio);
        let mut thumbnail_urls = get_urls_by_aspect(service_host, &self.id, aspect_kind);

        if let Some(url) = &self.thumbnail {
            thumbnail_urls.push(url.clone());
        }
        for Thumbnail { url } in self.thumbnails.as_deref().unwrap_or_default() {
            if let Some(url) = url.clone() {
                thumbnail_urls.push(url);
            }
        }
        thumbnail_urls
    }

    pub fn domain(&self) -> Option<String> {
        match self.webpage_url_domain.as_ref() {
            Some(domain) => Some(domain.clone()),
            None => match Url::parse(&self.original_url) {
                Ok(url) => match url.domain() {
                    Some(domain) => Some(domain.to_owned()),
                    None => None,
                },
                Err(_) => todo!(),
            },
        }
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct VideosInYT(Vec<Video>);

impl VideosInYT {
    pub fn new(videos: impl Into<Vec<Video>>) -> Self {
        Self(videos.into())
    }

    pub const fn len(&self) -> usize {
        self.0.len()
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
