use super::{combined_format, format};

use serde::Deserialize;
use std::{collections::VecDeque, ops::Deref, path::PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Thumbnail {
    pub filesize: Option<i64>,
    pub height: Option<f64>,
    pub id: Option<String>,
    pub preference: Option<i64>,
    pub url: Option<String>,
    pub width: Option<f64>,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone, Deserialize)]
pub struct VideoInYT {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub thumbnail: Option<String>,
    pub url: Option<String>,
    pub duration: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,

    formats: Vec<format::Any>,
}

impl VideoInYT {
    pub fn get_combined_formats(&self) -> combined_format::Formats<'_> {
        let mut format_kinds = vec![];

        for format in &self.formats {
            let Ok(format) = format.kind() else {
                continue;
            };

            format_kinds.push(format);
        }

        combined_format::Formats::from(format_kinds)
    }

    pub fn get_audio_formats(&self) -> format::Audios<'_> {
        let mut formats = vec![];

        for format in &self.formats {
            let Ok(format) = format.kind() else {
                continue;
            };

            if let format::Kind::Audio(format) = format {
                formats.push(format);
            }
        }

        format::Audios::from(formats)
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct VideosInYT(VecDeque<VideoInYT>);

impl VideosInYT {
    pub fn new(videos: impl Into<VecDeque<VideoInYT>>) -> Self {
        Self(videos.into())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Iterator for VideosInYT {
    type Item = VideoInYT;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.pop_front()
    }
}

impl Extend<VideoInYT> for VideosInYT {
    fn extend<T: IntoIterator<Item = VideoInYT>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl Deref for VideosInYT {
    type Target = VecDeque<VideoInYT>;

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
