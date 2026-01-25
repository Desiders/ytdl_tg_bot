use serde::Deserialize;
use std::{collections::BTreeMap, fmt, path::PathBuf};
use tempfile::TempDir;
use tracing::trace;
use url::Url;

use crate::utils::AspectKind;

#[derive(Debug, Clone)]
pub struct MediaFormat {
    pub format_id: String,
    pub format_note: Option<String>,
    pub ext: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub aspect_ratio: Option<f32>,
    pub filesize_approx: Option<u32>,
}

impl MediaFormat {
    #[inline]
    pub fn aspect_ration_kind(&self) -> Option<AspectKind> {
        self.aspect_ratio.map(AspectKind::new)
    }
}

impl fmt::Display for MediaFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} [{}]", self.format_id, self.ext)?;
        if let Some(note) = &self.format_note {
            write!(f, " - {}", note)?;
        }
        if let (Some(w), Some(h)) = (self.width, self.height) {
            write!(f, " {}x{}", w, h)?;
        }
        let size_mb = self.filesize_approx.unwrap_or(0) as f32 / (1024.0 * 1024.0);
        write!(f, " (~{:.2} MB)", size_mb)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Thumbnail {
    pub width: Option<i16>,
    pub height: Option<i16>,
    pub url: Url,
}

impl fmt::Display for Thumbnail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let size_str = match (self.width, self.height) {
            (Some(w), Some(h)) => format!("{}x{}", w, h),
            (Some(w), None) => format!("{}x?", w),
            (None, Some(h)) => format!("?x{}", h),
            (None, None) => "?x?".to_string(),
        };
        write!(f, "{} [{}]", self.url, size_str)
    }
}

#[derive(Debug, Clone)]
pub struct Media {
    pub id: String,
    pub display_id: Option<String>,
    pub original_url: Url,
    pub webpage_url: Url,
    pub title: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f32>,
    pub playlist_index: i16,
    pub thumbnail: Option<String>,
    pub thumbnails: Vec<Thumbnail>,
}

impl Media {
    pub fn get_thumb_urls(&self, aspect_kind: Option<AspectKind>) -> Vec<String> {
        let mut urls = match self.webpage_url.host_str() {
            Some(host) => {
                if host.contains("youtube") || host == "youtu.be" {
                    match aspect_kind {
                        Some(AspectKind::Vertical) => vec!["oardefault"],
                        Some(AspectKind::Sd) => vec!["sddefault", "0", "hqdefault"],
                        Some(AspectKind::Hd) => vec!["maxresdefault", "hq720", "maxres2"],
                        _ => vec![],
                    }
                    .into_iter()
                    .chain(Some("frame0"))
                    .map(|fragment| format!("https://i.ytimg.com/vi/{}/{fragment}.jpg", self.id))
                    .collect()
                } else {
                    vec![]
                }
            }
            None => vec![],
        };
        if let Some(thumb_url) = &self.thumbnail {
            urls.push(thumb_url.to_owned());
        }
        for thumb in &self.thumbnails {
            urls.push(thumb.url.to_string());
        }
        urls
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MediaWithFormat {
    pub id: String,
    pub display_id: Option<String>,
    pub original_url: Url,
    pub webpage_url: Url,
    pub title: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f32>,
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub thumbnails: Vec<Thumbnail>,

    pub format_id: String,
    pub format_note: Option<String>,
    pub ext: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub aspect_ratio: Option<f32>,
    pub filesize_approx: Option<u32>,
    pub filename: String,
    pub playlist_index: Option<i16>,
}

#[derive(Debug)]
pub struct Playlist {
    pub inner: Vec<(Media, Vec<MediaFormat>)>,
}

impl Playlist {
    pub fn new(media_with_formats: Vec<MediaWithFormat>) -> Self {
        use std::collections::btree_map::Entry::{Occupied, Vacant};

        trace!("{media_with_formats:?}");

        let mut inner = BTreeMap::new();

        for media_with_format in media_with_formats {
            match inner.entry(media_with_format.id.clone()) {
                Vacant(vacant_entry) => {
                    vacant_entry.insert(vec![media_with_format]);
                }
                Occupied(mut occupied_entry) => {
                    occupied_entry.get_mut().push(media_with_format);
                }
            }
        }

        let mut inner: Vec<(Media, Vec<MediaFormat>)> = inner
            .into_values()
            .map(|mut media_with_formats| {
                let first = media_with_formats.remove(0);
                let other = if media_with_formats.is_empty() {
                    vec![first.clone().into()]
                } else {
                    media_with_formats.into_iter().map(Into::into).collect()
                };
                (first.into(), other)
            })
            .collect();
        inner.sort_by_key(|(val, _)| val.playlist_index);

        Self { inner }
    }
}

#[derive(Debug)]
pub struct MediaInPlaylist {
    pub file_id: String,
    pub playlist_index: i16,
}

#[derive(Debug)]
pub struct MediaInFS {
    pub path: PathBuf,
    pub thumb_path: Option<PathBuf>,
    pub temp_dir: TempDir,
}

impl From<MediaWithFormat> for Media {
    fn from(
        MediaWithFormat {
            id,
            display_id,
            original_url,
            webpage_url,
            title,
            description,
            language,
            uploader,
            duration,
            playlist_index,
            thumbnail,
            thumbnails,
            ..
        }: MediaWithFormat,
    ) -> Self {
        Self {
            id,
            display_id,
            original_url,
            webpage_url,
            title,
            description,
            language,
            uploader,
            duration,
            playlist_index: playlist_index.unwrap_or(1),
            thumbnail,
            thumbnails,
        }
    }
}

impl From<MediaWithFormat> for MediaFormat {
    fn from(
        MediaWithFormat {
            format_id,
            format_note,
            ext,
            width,
            height,
            aspect_ratio,
            filesize_approx,
            ..
        }: MediaWithFormat,
    ) -> Self {
        Self {
            format_id,
            format_note,
            ext,
            width,
            height,
            aspect_ratio,
            filesize_approx,
        }
    }
}
