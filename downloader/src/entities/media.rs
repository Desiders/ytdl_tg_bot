use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fmt,
    hash::{Hash, Hasher},
};
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
    pub filesize_approx: Option<u64>,
}

impl MediaFormat {
    #[inline]
    pub fn aspect_ratio_kind(&self) -> Option<AspectKind> {
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} [{}]", self.format_id, self.ext)?;
        if let Some(note) = &self.format_note {
            write!(f, " - {note}")?;
        }
        if let (Some(w), Some(h)) = (self.width, self.height) {
            write!(f, " {w}x{h}")?;
        }
        let size_bytes = self.filesize_approx.unwrap_or(0);
        let whole_mb = size_bytes / (1024 * 1024);
        let fractional_mb = (size_bytes % (1024 * 1024)) * 100 / (1024 * 1024);
        write!(f, " (~{whole_mb}.{fractional_mb:02} MB)")?;
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
    pub direct_url: Option<Url>,
    pub title: Option<String>,
    pub language: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f32>,
    pub playlist_index: i16,
    pub thumbnail: Option<Url>,
    pub thumbnails: Vec<Thumbnail>,
    pub live_status: Option<Box<str>>,
    pub is_live: bool,
}

impl Media {
    pub fn is_active_livestream(&self) -> bool {
        self.is_live || matches!(self.live_status.as_deref(), Some("is_live" | "is_upcoming"))
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

#[derive(Debug, Clone, Deserialize)]
pub struct MediaWithFormat {
    pub id: String,
    pub display_id: Option<String>,
    pub webpage_url: Url,
    #[serde(default)]
    pub direct_url: Option<Url>,
    pub title: Option<String>,
    pub language: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f32>,
    pub thumbnail: Option<Url>,
    #[serde(default)]
    pub thumbnails: Vec<Thumbnail>,
    pub live_status: Option<Box<str>>,
    #[serde(default)]
    pub is_live: bool,

    pub format_id: String,
    pub format_note: Option<String>,
    pub ext: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub aspect_ratio: Option<f32>,
    pub filesize_approx: Option<u64>,
    pub playlist_index: Option<i16>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub vcodec: Option<String>,
    #[serde(default)]
    pub acodec: Option<String>,
}

impl MediaWithFormat {
    // Streamable only when it's a single pre-muxed HTTP(S) file that yt-dlp can write to stdout
    // as-is. Such formats (e.g. YouTube progressive) are already faststart MP4, so Telegram plays
    // and seeks them. HLS/DASH and `+` merges are excluded: a pipe can't produce a seekable file.
    pub fn is_progressive_streamable(&self) -> bool {
        !self.format_id.contains('+')
            && matches!(self.protocol.as_deref(), Some("https" | "http"))
            && self.vcodec.as_deref().is_some_and(|codec| codec != "none")
            && self.acodec.as_deref().is_some_and(|codec| codec != "none")
    }
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

impl From<MediaWithFormat> for Media {
    fn from(
        MediaWithFormat {
            id,
            display_id,
            webpage_url,
            direct_url,
            title,
            language,
            uploader,
            duration,
            playlist_index,
            thumbnail,
            thumbnails,
            live_status,
            is_live,
            ..
        }: MediaWithFormat,
    ) -> Self {
        Self {
            id,
            display_id,
            webpage_url,
            direct_url,
            title,
            language,
            uploader,
            duration,
            playlist_index: playlist_index.unwrap_or(1),
            thumbnail,
            thumbnails,
            live_status,
            is_live,
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

#[cfg(test)]
mod tests {
    use super::Media;
    use url::Url;

    fn media(live_status: Option<&str>, is_live: bool) -> Media {
        Media {
            id: "id".into(),
            display_id: None,
            webpage_url: Url::parse("https://www.youtube.com/watch?v=test").unwrap(),
            direct_url: None,
            title: None,
            language: None,
            uploader: None,
            duration: None,
            playlist_index: 1,
            thumbnail: None,
            thumbnails: vec![],
            live_status: live_status.map(Into::into),
            is_live,
        }
    }

    #[test]
    fn active_livestream_when_is_live_flag_is_true() {
        assert!(media(None, true).is_active_livestream());
    }

    #[test]
    fn active_livestream_when_live_status_is_live() {
        assert!(media(Some("is_live"), false).is_active_livestream());
    }

    #[test]
    fn active_livestream_when_live_status_is_upcoming() {
        assert!(media(Some("is_upcoming"), false).is_active_livestream());
    }

    #[test]
    fn not_active_livestream_when_was_live() {
        assert!(!media(Some("was_live"), false).is_active_livestream());
    }
}
