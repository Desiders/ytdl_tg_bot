use super::{AnyFormat, CombinedFormats};

use serde::Deserialize;
use std::{collections::VecDeque, ops::Deref};

#[derive(Debug, Clone, Deserialize)]
pub struct Thumbnail {
    pub filesize: Option<i64>,
    pub height: Option<f64>,
    pub id: Option<String>,
    pub preference: Option<i64>,
    pub url: Option<String>,
    pub width: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub thumbnail: Option<String>,
    pub thumbnails: Option<Vec<Thumbnail>>,

    formats: Vec<AnyFormat>,
}

impl Video {
    pub fn get_combined_formats<'a>(&'a self) -> CombinedFormats<'a> {
        let mut format_kinds = vec![];

        for format in self.formats.iter() {
            let Ok(format) = format.kind() else {
                continue;
            };

            format_kinds.push(format);
        }

        CombinedFormats::from(format_kinds)
    }

    pub fn get_best_thumbnail(&self) -> Option<&Thumbnail> {
        let Some(thumbnails) = self.thumbnails.as_ref() else {
            return None;
        };

        thumbnails
            .iter()
            .filter(|thumbnail| thumbnail.url.is_some())
            .max_by_key(|thumbnail| thumbnail.filesize)
    }

    pub fn get_best_thumbnail_url(&self) -> Option<&str> {
        self.get_best_thumbnail().and_then(|thumbnail| thumbnail.url.as_deref())
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Videos(pub VecDeque<Video>);

impl Videos {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Iterator for Videos {
    type Item = Video;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.pop_front()
    }
}

impl Extend<Video> for Videos {
    fn extend<T: IntoIterator<Item = Video>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl Deref for Videos {
    type Target = VecDeque<Video>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
