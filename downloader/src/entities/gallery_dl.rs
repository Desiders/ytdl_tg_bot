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
    // Set for snapsave images: fetch `direct_url` directly instead of re-extracting via gallery-dl.
    #[serde(default)]
    pub direct: bool,
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
            protocol: None,
            vcodec: None,
            acodec: None,
            direct: false,
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
        self.extract_string(&["id", "tweet_id", "post_id"])
            .or_else(|| self.extract_u64(&["id", "tweet_id", "post_id"]).map(|v| v.to_string()))
    }

    pub fn extract_author(&self) -> Option<String> {
        if let Some(author) = self.extract_string(&["author", "uploader"]) {
            return Some(author);
        }
        let user = self.metadata.as_ref()?.get("user")?;
        user.get("name")
            .or_else(|| user.get("nick"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }

    pub fn extract_dimensions(&self) -> Option<(u32, u32)> {
        let width = self.extract_u64(&["width", "image_width", "imageWidth"])?;
        let height = self.extract_u64(&["height", "image_height", "imageHeight"])?;
        Some((u32::try_from(width).ok()?, u32::try_from(height).ok()?))
    }

    pub fn into_raw_photo_info(self, request_url: &Url) -> Result<RawPhotoInfo, DroppedNonPhotoEntry> {
        let direct_url = self.file_url.clone();
        let ext = self
            .extract_extension()
            .or_else(|| {
                Path::new(direct_url.path())
                    .extension()
                    .and_then(OsStr::to_str)
                    .map(ToOwned::to_owned)
            })
            .map_or_else(|| "jpg".to_owned(), |ext| ext.to_ascii_lowercase());
        if !is_photo_extension(&ext) {
            return Err(DroppedNonPhotoEntry { url: direct_url, ext });
        }

        let display_id = self.extract_display_id();
        let id = self
            .extract_id()
            .or_else(|| display_id.clone())
            .unwrap_or_else(|| direct_url.as_str().to_owned());
        let (width, height) = self
            .extract_dimensions()
            .map_or((None, None), |(width, height)| (Some(i64::from(width)), Some(i64::from(height))));

        Ok(RawPhotoInfo {
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
            playlist_index: 0,
            direct: false,
        })
    }

    fn extract_string(&self, keys: &[&str]) -> Option<String> {
        let meta = self.metadata.as_ref()?;
        keys.iter()
            .find_map(|key| meta.get(*key).and_then(Value::as_str))
            .map(ToOwned::to_owned)
    }

    fn extract_u64(&self, keys: &[&str]) -> Option<u64> {
        let meta = self.metadata.as_ref()?;
        keys.iter().find_map(|key| meta.get(*key).and_then(Value::as_u64))
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
        ["webpage_url", "page_url", "source_url", "post_url"]
            .iter()
            .find_map(|key| meta.get(*key).and_then(Value::as_str).and_then(|raw| Url::parse(raw).ok()))
    }
}

fn is_photo_extension(ext: &str) -> bool {
    matches!(
        ext,
        "jpg" | "jpeg" | "jfif" | "png" | "webp" | "gif" | "heic" | "heif" | "avif" | "bmp" | "tiff" | "tif" | "jxl" | "svg"
    )
}

#[derive(Debug, thiserror::Error)]
#[error("Non-photo extension `{ext}`: {url}")]
pub struct DroppedNonPhotoEntry {
    pub url: Url,
    pub ext: String,
}
