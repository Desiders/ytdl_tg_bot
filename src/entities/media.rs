use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fmt,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use url::Url;

use crate::{config::TrackingParamsConfig, utils::AspectKind};

#[derive(Debug, Clone, Deserialize)]
pub struct ShortMedia {
    pub id: String,
    pub title: Option<String>,
    pub thumbnail: Option<Url>,
}

#[derive(Debug, Clone)]
pub struct MediaFormat {
    pub format_id: String,
    pub format_note: Option<String>,
    pub ext: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub aspect_ratio: Option<f32>,
    pub filesize_approx: Option<u64>,
}

impl MediaFormat {
    #[inline]
    pub fn aspect_ration_kind(&self) -> Option<AspectKind> {
        self.aspect_ratio.map(AspectKind::new)
    }
}

impl PartialEq for MediaFormat {
    fn eq(&self, other: &Self) -> bool {
        self.format_id == other.format_id && self.ext == other.ext && self.width == other.width && self.height == other.height
    }
}

impl Eq for MediaFormat {}

impl Hash for MediaFormat {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.format_id.hash(state);
    }
}

impl fmt::Display for MediaFormat {
    #[allow(clippy::cast_precision_loss)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} [{}]", self.format_id, self.ext)?;
        if let Some(note) = &self.format_note {
            write!(f, " - {note}")?;
        }
        if let (Some(w), Some(h)) = (self.width, self.height) {
            write!(f, " {w}x{h}")?;
        }
        let size_mb = self.filesize_approx.unwrap_or(0) as f32 / (1024.0 * 1024.0);
        write!(f, " (~{size_mb:.2} MB)")?;
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
            (Some(w), Some(h)) => format!("{w}x{h}"),
            (Some(w), None) => format!("{w}x?"),
            (None, Some(h)) => format!("?x{h}"),
            (None, None) => "?x?".to_string(),
        };
        write!(f, "{} [{}]", self.url, size_str)
    }
}

#[derive(Debug, Clone)]
pub struct Media {
    pub id: String,
    pub display_id: Option<String>,
    pub webpage_url: Url,
    pub title: Option<String>,
    pub language: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f32>,
    pub playlist_index: i16,
    pub thumbnail: Option<Url>,
    pub thumbnails: Vec<Thumbnail>,
}

impl Media {
    pub fn remove_url_tracking_params(&mut self, cfg: &TrackingParamsConfig) {
        let params = self
            .webpage_url
            .query_pairs()
            .filter(|(key, _)| cfg.params.iter().all(|val| **val != **key))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect::<Box<[_]>>();
        self.webpage_url.query_pairs_mut().clear().extend_pairs(params);
    }

    pub fn get_thumb_urls(&self, aspect_kind: Option<AspectKind>) -> Vec<Url> {
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
                    .map(|fragment| Url::parse(&format!("https://i.ytimg.com/vi/{}/{fragment}.jpg", self.id)).unwrap())
                    .collect()
                } else {
                    vec![]
                }
            }
            None => vec![],
        };
        if let Some(thumb_url) = &self.thumbnail {
            urls.push(thumb_url.clone());
        }
        for thumb in &self.thumbnails {
            urls.push(thumb.url.clone());
        }
        urls
    }

    fn get_thumb_url_jpg(&self) -> Option<&Url> {
        fn is_jpg_or_jpeg(url: &Url) -> bool {
            Path::new(url.path())
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg"))
        }

        if let Some(url) = &self.thumbnail {
            if is_jpg_or_jpeg(url) {
                return Some(url);
            }
        }
        self.thumbnails.iter().map(|val| &val.url).find(|url| is_jpg_or_jpeg(url))
    }
}

impl fmt::Display for Media {
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(title) = &self.title {
            write!(f, "{title}")?;
        } else {
            write!(f, "Media {}", self.id)?;
        }
        if let Some(display_id) = &self.display_id {
            write!(f, " ({display_id})")?;
        }
        if let Some(uploader) = &self.uploader {
            write!(f, " — {uploader}")?;
        }
        if let Some(duration) = self.duration {
            let mins = (duration / 60.0).floor() as u32;
            let secs = (duration % 60.0).round() as u32;
            write!(f, " [{mins}:{secs:02}]")?;
        }
        if let Some(lang) = &self.language {
            write!(f, " [{lang}]")?;
        }
        if self.playlist_index > 0 {
            write!(f, " (#{})", self.playlist_index)?;
        }

        Ok(())
    }
}

impl From<Media> for ShortMedia {
    fn from(media: Media) -> Self {
        let thumbnail = media.get_thumb_url_jpg().cloned();
        Self {
            id: media.id,
            title: media.title,
            thumbnail,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MediaWithFormat {
    pub id: String,
    pub display_id: Option<String>,
    pub webpage_url: Url,
    pub title: Option<String>,
    pub language: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f32>,
    pub thumbnail: Option<Url>,
    #[serde(default)]
    pub thumbnails: Vec<Thumbnail>,

    pub format_id: String,
    pub format_note: Option<String>,
    pub ext: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub aspect_ratio: Option<f32>,
    pub filesize_approx: Option<u64>,
    pub playlist_index: Option<i16>,
}

pub type RawMediaWithFormat = String;

#[derive(Debug)]
pub struct Playlist {
    pub inner: Vec<(Media, Vec<(MediaFormat, RawMediaWithFormat)>)>,
}

impl Playlist {
    pub fn new(media_with_formats: Vec<(MediaWithFormat, RawMediaWithFormat)>) -> Self {
        use std::collections::btree_map::Entry::{Occupied, Vacant};

        let mut inner = BTreeMap::new();

        for (media_with_format, raw) in media_with_formats {
            match inner.entry(media_with_format.id.clone()) {
                Vacant(vacant_entry) => {
                    vacant_entry.insert(vec![(media_with_format, raw)]);
                }
                Occupied(mut occupied_entry) => {
                    occupied_entry.get_mut().push((media_with_format, raw));
                }
            }
        }

        let mut inner: Vec<(Media, Vec<(MediaFormat, RawMediaWithFormat)>)> = inner
            .into_values()
            .map(|mut media_with_formats| {
                let (first_format, raw) = media_with_formats.remove(0);
                let mut formats = vec![];
                if media_with_formats.is_empty() {
                    formats.push((first_format.clone().into(), raw));
                } else {
                    for (media_with_format, raw) in media_with_formats {
                        let format: MediaFormat = media_with_format.into();
                        if formats.contains(&(format.clone(), raw.clone())) {
                            continue;
                        }
                        formats.push((format, raw));
                    }
                }
                (first_format.into(), formats)
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
    pub webpage_url: Option<Url>,
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
            webpage_url,
            title,
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
            webpage_url,
            title,
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
