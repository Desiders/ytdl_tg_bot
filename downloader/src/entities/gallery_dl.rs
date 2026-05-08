use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{ffi::OsStr, path::Path};
use url::Url;

use crate::entities::MediaWithFormat;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawPhotoInfo {
    pub id: String,
    pub display_id: Option<String>,
    pub webpage_url: Url,
    pub direct_url: Url,
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub ext: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub filesize_approx: Option<u64>,
    pub playlist_index: i16,
}

impl RawPhotoInfo {
    pub fn into_playlist_entry(self) -> Result<(MediaWithFormat, String), serde_json::Error> {
        let raw = serde_json::to_string(&self)?;
        Ok((self.into(), raw))
    }
}

impl From<RawPhotoInfo> for MediaWithFormat {
    fn from(raw_info: RawPhotoInfo) -> Self {
        MediaWithFormat {
            id: raw_info.id,
            display_id: raw_info.display_id,
            webpage_url: raw_info.webpage_url,
            direct_url: Some(raw_info.direct_url),
            title: raw_info.title,
            language: None,
            uploader: raw_info.uploader,
            duration: None,
            thumbnail: None,
            thumbnails: vec![],
            live_status: None,
            is_live: false,
            format_id: "photo".to_owned(),
            format_note: Some("photo".to_owned()),
            ext: raw_info.ext,
            width: raw_info.width,
            height: raw_info.height,
            aspect_ratio: None,
            filesize_approx: raw_info.filesize_approx,
            playlist_index: Some(raw_info.playlist_index),
        }
    }
}

#[derive(Debug)]
pub struct GalleryDlEntry {
    pub file_url: Url,
    pub metadata: Option<Value>,
}

impl GalleryDlEntry {
    pub fn extract_id(&self) -> Option<String> {
        if let Some(meta) = &self.metadata {
            if let Some(id) = meta.get("id").and_then(Value::as_str) {
                return Some(id.to_owned());
            }
            if let Some(id) = meta.get("id").and_then(Value::as_u64) {
                return Some(id.to_string());
            }
            if let Some(id) = meta.get("tweet_id").and_then(Value::as_str) {
                return Some(id.to_owned());
            }
            if let Some(id) = meta.get("tweet_id").and_then(Value::as_u64) {
                return Some(id.to_string());
            }
            if let Some(id) = meta.get("post_id").and_then(Value::as_str) {
                return Some(id.to_owned());
            }
            if let Some(id) = meta.get("post_id").and_then(Value::as_u64) {
                return Some(id.to_string());
            }
        }
        None
    }

    pub fn extract_author(&self) -> Option<String> {
        if let Some(meta) = &self.metadata {
            if let Some(author) = meta.get("author").and_then(Value::as_str) {
                return Some(author.to_owned());
            }
            if let Some(user) = meta.get("user") {
                if let Some(name) = user.get("name").and_then(Value::as_str) {
                    return Some(name.to_owned());
                }
                if let Some(nick) = user.get("nick").and_then(Value::as_str) {
                    return Some(nick.to_owned());
                }
            }
            if let Some(uploader) = meta.get("uploader").and_then(Value::as_str) {
                return Some(uploader.to_owned());
            }
        }
        None
    }

    pub fn extract_dimensions(&self) -> Option<(u32, u32)> {
        if let Some(meta) = &self.metadata {
            let width = meta.get("width").or_else(|| meta.get("image_width")).and_then(Value::as_u64);
            let height = meta.get("height").or_else(|| meta.get("image_height")).and_then(Value::as_u64);

            if let (Some(w), Some(h)) = (width, height) {
                return Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?));
            }
        }
        None
    }

    pub fn into_raw_photo_info(self, request_url: &Url, playlist_index: i16) -> Option<RawPhotoInfo> {
        let direct_url = self.file_url.clone();
        let ext = self.extract_extension().or_else(|| {
            Path::new(direct_url.path())
                .extension()
                .and_then(OsStr::to_str)
                .map(ToOwned::to_owned)
        })?;
        let display_id = self.extract_display_id();
        let id = self
            .extract_id()
            .or_else(|| display_id.clone())
            .unwrap_or_else(|| direct_url.as_str().to_owned());
        let (width, height) = self
            .extract_dimensions()
            .map_or((None, None), |(width, height)| (Some(i64::from(width)), Some(i64::from(height))));

        Some(RawPhotoInfo {
            id,
            display_id,
            webpage_url: self.extract_webpage_url().unwrap_or_else(|| request_url.clone()),
            direct_url,
            title: self.extract_title(),
            uploader: self.extract_author(),
            ext,
            width,
            height,
            filesize_approx: self.extract_filesize(),
            playlist_index,
        })
    }

    fn extract_string(&self, keys: &[&str]) -> Option<String> {
        let meta = self.metadata.as_ref()?;
        for key in keys {
            if let Some(value) = meta.get(*key).and_then(Value::as_str) {
                return Some(value.to_owned());
            }
        }
        None
    }

    fn extract_u64(&self, keys: &[&str]) -> Option<u64> {
        let meta = self.metadata.as_ref()?;
        for key in keys {
            if let Some(value) = meta.get(*key).and_then(Value::as_u64) {
                return Some(value);
            }
        }
        None
    }

    fn extract_display_id(&self) -> Option<String> {
        self.extract_string(&["display_id", "filename", "name", "slug"])
    }

    fn extract_extension(&self) -> Option<String> {
        self.extract_string(&["extension", "ext"])
    }

    fn extract_title(&self) -> Option<String> {
        self.extract_string(&["title", "description", "content", "caption"])
    }

    fn extract_filesize(&self) -> Option<u64> {
        self.extract_u64(&["filesize", "file_size", "size"])
    }

    fn extract_webpage_url(&self) -> Option<Url> {
        let meta = self.metadata.as_ref()?;
        for key in ["webpage_url", "page_url", "source_url", "post_url"] {
            if let Some(url) = meta.get(key).and_then(Value::as_str).and_then(|raw| Url::parse(raw).ok()) {
                return Some(url);
            }
        }
        None
    }
}
