use super::{combined_format, format};

use serde::Deserialize;
use std::{collections::VecDeque, ops::Deref, path::PathBuf};

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
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct VideosInYT(VecDeque<Video>);

impl VideosInYT {
    pub fn new(videos: impl Into<VecDeque<Video>>) -> Self {
        Self(videos.into())
    }
}

impl Iterator for VideosInYT {
    type Item = Video;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.pop_front()
    }
}

impl Extend<Video> for VideosInYT {
    fn extend<T: IntoIterator<Item = Video>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl Deref for VideosInYT {
    type Target = VecDeque<Video>;

    fn deref(&self) -> &Self::Target {
        &self.0
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
}

impl VideoInFS {
    pub fn new(path: impl Into<PathBuf>, thumbnail_path: Option<PathBuf>) -> Self {
        Self {
            path: path.into(),
            thumbnail_path,
        }
    }
}
