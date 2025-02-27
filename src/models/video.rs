use super::{combined_format, format};

use serde::Deserialize;
use std::{collections::VecDeque, ops::Deref, path::PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Thumbnail {
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub url: Option<String>,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone, Deserialize)]
pub struct VideoInYT {
    pub id: String,
    pub title: Option<String>,
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

impl VideoInYT {
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
            };
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
            };
        }

        format::Audios::from(formats)
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn thumbnail(&self) -> Option<&str> {
        let (video_width, video_height) = match (self.width, self.height) {
            (Some(w), Some(h)) => (w as f32, h as f32),
            _ => {
                return self.thumbnail.as_deref().or(self
                    .thumbnails
                    .as_deref()
                    .and_then(|thumbnails| thumbnails[thumbnails.len() - 1].url.as_deref()))
            }
        };
        let video_ratio = video_width / video_height;

        let mut best_thumbnail = None;
        let mut best_score = f32::MAX;

        for thumb in self.thumbnails.as_deref().unwrap_or_default() {
            let (Some(thumb_width), Some(thumb_height)) = (thumb.width, thumb.height) else {
                continue;
            };

            let thumb_ratio = thumb_width as f32 / thumb_height as f32;
            let ratio_diff = (video_ratio - thumb_ratio).abs();
            let size_diff = ((video_width as i32 - thumb_width as i32).abs() + (video_height as i32 - thumb_height as i32).abs()) as f32;

            let score = ratio_diff * 10.0 + size_diff;

            if score < best_score {
                best_score = score;
                best_thumbnail = Some(thumb);
            }
        }

        best_thumbnail
            .and_then(|thumbnail| thumbnail.url.as_deref())
            .or(self.thumbnail.as_deref().or(self
                .thumbnails
                .as_deref()
                .and_then(|thumbnails| thumbnails[thumbnails.len() - 1].url.as_deref())))
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct VideosInYT(VecDeque<VideoInYT>);

impl VideosInYT {
    pub fn new(videos: impl Into<VecDeque<VideoInYT>>) -> Self {
        Self(videos.into())
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
