use crate::{
    config::GalleryDlConfig,
    entities::{GalleryDlEntry, Playlist, Range, RawPhotoInfo},
};

use serde_json::{Map, Value};
use std::{
    io,
    path::Path,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::time;
use tracing::{debug, error, instrument, trace, warn};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum ParseJsonErrorKind {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("URL error: {0}")]
    Url(#[from] url::ParseError),
}

#[derive(Debug, thiserror::Error)]
pub enum GetInfoErrorKind {
    #[error(transparent)]
    GalleryDlJson(#[from] ParseJsonErrorKind),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Gallery-dl output did not contain downloadable photos")]
    EmptyEntries,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadErrorKind {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

fn parse_gallery_dl_json(input: &[u8]) -> Result<Vec<GalleryDlEntry>, ParseJsonErrorKind> {
    let root: Value = serde_json::from_slice(input)?;
    match root {
        Value::Array(events) => parse_gallery_dl_events(events),
        Value::Object(_) => entries_from_metadata(&root),
        _ => Ok(vec![]),
    }
}

fn parse_gallery_dl_events(events: Vec<Value>) -> Result<Vec<GalleryDlEntry>, ParseJsonErrorKind> {
    let mut results = Vec::with_capacity(1);
    let mut current_metadata = None;

    for event in events {
        let Some(event) = event.as_array() else {
            continue;
        };
        match event.first().and_then(Value::as_u64) {
            Some(2) => {
                current_metadata = event.get(1).cloned();
            }
            Some(3) => {
                let Some(file_url) = event.get(1).and_then(Value::as_str) else {
                    continue;
                };
                let metadata = event.get(2).cloned().unwrap_or_default();
                results.push(GalleryDlEntry {
                    file_url: Url::parse(file_url)?,
                    metadata: merge_metadata(current_metadata.as_ref(), metadata),
                });
            }
            _ => {}
        }
    }

    if results.is_empty() {
        if let Some(metadata) = current_metadata {
            results = entries_from_metadata(&metadata)?;
        }
    }

    Ok(results)
}

fn entries_from_metadata(metadata: &Value) -> Result<Vec<GalleryDlEntry>, ParseJsonErrorKind> {
    let mut results = Vec::with_capacity(1);

    if let Some(file_url) = direct_url_from_metadata(metadata)? {
        results.push(GalleryDlEntry {
            file_url,
            metadata: Some(metadata.clone()),
        });
    }

    let Some(object) = metadata.as_object() else {
        return Ok(results);
    };

    for key in ["files", "images", "media", "attachments", "photos"] {
        match object.get(key) {
            Some(Value::Array(items)) => {
                for item in items {
                    if let Some(file_url) = direct_url_from_metadata(item)? {
                        results.push(GalleryDlEntry {
                            file_url,
                            metadata: merge_metadata(Some(metadata), item.clone()),
                        });
                    }
                }
            }
            Some(item @ Value::Object(_)) => {
                if let Some(file_url) = direct_url_from_metadata(item)? {
                    results.push(GalleryDlEntry {
                        file_url,
                        metadata: merge_metadata(Some(metadata), item.clone()),
                    });
                }
            }
            _ => {}
        }
    }

    Ok(results)
}

fn direct_url_from_metadata(metadata: &Value) -> Result<Option<Url>, ParseJsonErrorKind> {
    let Some(object) = metadata.as_object() else {
        return Ok(None);
    };

    for key in [
        "url",
        "file_url",
        "direct_url",
        "image",
        "image_url",
        "media_url",
        "original",
        "src",
    ] {
        if let Some(raw) = object.get(key).and_then(Value::as_str) {
            return Ok(Some(Url::parse(raw)?));
        }
    }

    Ok(None)
}

fn merge_metadata(base: Option<&Value>, item: Value) -> Option<Value> {
    match (base, item) {
        (Some(Value::Object(base)), Value::Object(item)) => {
            let mut merged = Map::with_capacity(base.len() + item.len());
            merged.extend(base.clone());
            merged.extend(item);
            Some(Value::Object(merged))
        }
        (_, Value::Object(item)) if item.is_empty() => base.cloned(),
        (_, Value::Null) => base.cloned(),
        (_, item) => Some(item),
    }
}

fn build_range_string(range: &Range) -> String {
    range.to_range_string()
}

#[instrument(skip_all)]
pub async fn get_media_info(
    search: &str,
    gallery_dl_cfg: &GalleryDlConfig,
    playlist_range: &Range,
    timeout: u64,
    cookie_path: Option<&Path>,
) -> Result<Playlist, GetInfoErrorKind> {
    let range = build_range_string(playlist_range);
    let request_url = Url::parse(search).map_err(io::Error::other)?;
    let mut entries = get_playlist_entries(search, &request_url, gallery_dl_cfg, &range, timeout, cookie_path, false).await?;
    if entries.is_empty() {
        debug!(url = %search, "Gallery-dl dump-json did not produce photo entries; retrying with resolve-json");
        entries = get_playlist_entries(search, &request_url, gallery_dl_cfg, &range, timeout, cookie_path, true).await?;
    }

    if entries.is_empty() {
        return Err(GetInfoErrorKind::EmptyEntries);
    }

    debug!(entries = entries.len(), "Parsed gallery-dl media info");
    Ok(Playlist::new(entries))
}

async fn get_playlist_entries(
    search: &str,
    request_url: &Url,
    gallery_dl_cfg: &GalleryDlConfig,
    range: &str,
    timeout: u64,
    cookie_path: Option<&Path>,
    resolve_urls: bool,
) -> Result<Vec<(crate::entities::MediaWithFormat, String)>, GetInfoErrorKind> {
    let Some(stdout) = run_gallery_dl_json(search, gallery_dl_cfg, range, timeout, cookie_path, resolve_urls).await? else {
        return Ok(vec![]);
    };

    parse_gallery_dl_json(&stdout)?
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.into_raw_photo_info(request_url, i16::try_from(index + 1).unwrap_or(i16::MAX)))
        .map(RawPhotoInfo::into_playlist_entry)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

async fn run_gallery_dl_json(
    search: &str,
    gallery_dl_cfg: &GalleryDlConfig,
    range: &str,
    timeout: u64,
    cookie_path: Option<&Path>,
    resolve_urls: bool,
) -> Result<Option<Vec<u8>>, GetInfoErrorKind> {
    let json_flag = if resolve_urls { "--resolve-json" } else { "--dump-json" };
    let mut args = vec![
        json_flag,
        "--simulate",
        "--no-input",
        "--config-ignore",
        "--no-colors",
        "--range",
        range,
    ];

    let cookie_path = cookie_path.map(|path| path.to_string_lossy());
    if let Some(cookie_path) = cookie_path.as_deref() {
        args.push("--cookies");
        args.push(cookie_path);
    }

    args.push("--");
    args.push(search);

    trace!(?args, resolve_urls, "Gallery-dl args");

    let child = create_gallery_dl_command(gallery_dl_cfg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    match time::timeout(Duration::from_secs(timeout), child.wait_with_output()).await {
        Ok(Ok(Output { status, stdout, stderr })) => {
            let stderr = String::from_utf8_lossy(&stderr);
            if !status.success() {
                error!("{stderr}");
                return match status.code() {
                    Some(code) => Err(io::Error::other(format!("Gallery-dl exited with code {code} and message: {stderr}")).into()),
                    None => Err(io::Error::other(format!("Gallery-dl exited with and message: {stderr}")).into()),
                };
            }

            if !stderr.trim().is_empty() {
                warn!("{stderr}");
            }
            if stdout.iter().all(u8::is_ascii_whitespace) {
                warn!(
                    original_url = %search,
                    stderr = %stderr.trim(),
                    resolve_urls,
                    "Gallery-dl returned empty JSON output"
                );
                return Ok(None);
            }

            Ok(Some(stdout))
        }
        Ok(Err(err)) => Err(err.into()),
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "Gallery-dl timed out").into()),
    }
}

#[instrument(skip_all)]
pub async fn download_media(
    search: &str,
    raw_info: &RawPhotoInfo,
    max_filesize: u64,
    output_dir_path: &Path,
    gallery_dl_cfg: &GalleryDlConfig,
    timeout: u64,
    cookie_path: Option<&Path>,
) -> Result<(), DownloadErrorKind> {
    let max_filesize = max_filesize.to_string();
    let output_dir_path = output_dir_path.to_string_lossy();
    let filter = format!(
        "url == {}",
        serde_json::to_string(raw_info.direct_url.as_str()).expect("valid JSON string")
    );
    let file_name = format!("media.{}", raw_info.ext);

    let mut args = vec![
        "--no-input",
        "--config-ignore",
        "--no-colors",
        "--no-part",
        "--no-skip",
        "--filesize-max",
        &max_filesize,
        "--directory",
        &output_dir_path,
        "--filename",
        &file_name,
        "--filter",
        &filter,
    ];

    let cookie_path = cookie_path.map(|path| path.to_string_lossy());
    if let Some(cookie_path) = cookie_path.as_deref() {
        args.push("--cookies");
        args.push(cookie_path);
    }

    args.push("--");
    args.push(search);

    trace!(?args, "Gallery-dl args");

    let child = create_gallery_dl_command(gallery_dl_cfg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    match time::timeout(Duration::from_secs(timeout), child.wait_with_output()).await {
        Ok(Ok(Output { status, stderr, .. })) => {
            let stderr = String::from_utf8_lossy(&stderr);
            if status.success() {
                if !stderr.trim().is_empty() {
                    warn!("{stderr}");
                }
                Ok(())
            } else {
                error!("{stderr}");
                match status.code() {
                    Some(code) => Err(io::Error::other(format!("Gallery-dl exited with code {code} and message: {stderr}")).into()),
                    None => Err(io::Error::other(format!("Gallery-dl exited with and message: {stderr}")).into()),
                }
            }
        }
        Ok(Err(err)) => Err(err.into()),
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "Gallery-dl timed out").into()),
    }
}

fn create_gallery_dl_command(gallery_dl_cfg: &GalleryDlConfig) -> tokio::process::Command {
    let (program, base_args) = gallery_dl_cfg.command_parts();
    let mut command = tokio::process::Command::new(program);
    command.args(base_args);
    command
}

#[cfg(test)]
mod tests {
    use super::parse_gallery_dl_json;
    use serde_json::Value;
    use url::Url;

    #[test]
    fn parses_gallery_dl_event_array() {
        let input = br#"[
            [2, {"id": 42, "title": "Post"}],
            [3, "https://cdn.example/a.jpg", {"width": 640, "height": 480}],
            [3, "https://cdn.example/b.jpg", {}]
        ]"#;

        let entries = parse_gallery_dl_json(input).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].file_url.as_str(), "https://cdn.example/a.jpg");
        assert_eq!(
            entries[0].metadata.as_ref().and_then(|metadata| metadata.get("title")),
            Some(&Value::String("Post".to_owned()))
        );
        assert_eq!(
            entries[0].metadata.as_ref().and_then(|metadata| metadata.get("width")),
            Some(&Value::Number(640.into()))
        );
        assert_eq!(entries[1].file_url.as_str(), "https://cdn.example/b.jpg");
        assert_eq!(
            entries[1].metadata.as_ref().and_then(|metadata| metadata.get("title")),
            Some(&Value::String("Post".to_owned()))
        );
    }

    #[test]
    fn parses_metadata_only_event_array_when_it_contains_direct_url() {
        let input = br#"[
            [2, {"id": 42, "title": "Post", "url": "https://cdn.example/a.jpg"}]
        ]"#;

        let entries = parse_gallery_dl_json(input).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_url.as_str(), "https://cdn.example/a.jpg");
        assert_eq!(
            entries[0].metadata.as_ref().and_then(|metadata| metadata.get("title")),
            Some(&Value::String("Post".to_owned()))
        );
    }

    #[test]
    fn parses_root_metadata_object_when_it_contains_media_items() {
        let input = br#"{
            "id": 42,
            "title": "Post",
            "media": [{"url": "https://cdn.example/a.jpg"}, {"url": "https://cdn.example/b.jpg"}]
        }"#;

        let entries = parse_gallery_dl_json(input).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].file_url.as_str(), "https://cdn.example/a.jpg");
        assert_eq!(entries[1].file_url.as_str(), "https://cdn.example/b.jpg");
    }

    #[test]
    fn parses_vk_file_event_into_photo_info() {
        let input = br#"[
            [2, {"category": "vk", "wall": {"id": "218483"}}],
            [3, "https://sun9-86.userapi.com/s/v1/ig2/image.jpg?quality=95&cs=720x0", {
                "category": "vk",
                "extension": "jpg",
                "filename": "image",
                "height": 907,
                "id": "457284331",
                "url": "https://sun9-86.userapi.com/s/v1/ig2/image.jpg?quality=95&cs=720x0",
                "width": 720
            }]
        ]"#;
        let request_url = Url::parse("https://vk.com/wall90880680_218483").unwrap();

        let entries = parse_gallery_dl_json(input).unwrap();
        let raw = entries.into_iter().next().unwrap().into_raw_photo_info(&request_url, 1).unwrap();

        assert_eq!(raw.id, "457284331");
        assert_eq!(raw.display_id.as_deref(), Some("image"));
        assert_eq!(raw.ext, "jpg");
        assert_eq!(raw.width, Some(720));
        assert_eq!(raw.height, Some(907));
        assert_eq!(
            raw.direct_url.as_str(),
            "https://sun9-86.userapi.com/s/v1/ig2/image.jpg?quality=95&cs=720x0"
        );
    }
}
