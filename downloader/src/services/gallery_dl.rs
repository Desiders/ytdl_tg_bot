use crate::{
    config::GalleryDlConfig,
    entities::{GalleryDlEntry, Playlist, Range, RawPhotoInfo},
    utils::process_exit_error,
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
    #[error("Gallery-dl JSON root must be an array of events")]
    UnexpectedRoot,
    #[error("{0}")]
    Extraction(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GetInfoErrorKind {
    #[error(transparent)]
    GalleryDlJson(#[from] ParseJsonErrorKind),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    /// An error gallery-dl logged to stderr while still exiting successfully (e.g. "Requested post
    /// not available"). Carries the logged message.
    #[error("{0}")]
    Extraction(String),
    #[error("Gallery-dl output did not contain downloadable photos")]
    EmptyEntries,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadErrorKind {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

/// Parses gallery-dl's `--dump-json` / `--resolve-json` output: a `DataJob` array of message
/// tuples whose first element is the identifier. In gallery-dl 1.32 a `DataJob` only emits:
/// - `[2, metadata]` — Directory: post-level metadata for the files that follow.
/// - `[3, url, metadata]` — Url: a downloadable file and its metadata.
/// - `[6, url, metadata]` — Queue: an external URL for another extractor; the `--resolve-json` pass
///   follows these and re-emits Url/Directory messages, so we don't act on them here.
/// - `[-1, {error, message}]` — a `DataJob` error record (written when extraction raises), not a
///   real message type; we surface its message.
fn parse_gallery_dl_json(input: &[u8]) -> Result<Vec<GalleryDlEntry>, ParseJsonErrorKind> {
    let Value::Array(events) = serde_json::from_slice(input)? else {
        return Err(ParseJsonErrorKind::UnexpectedRoot);
    };

    let mut results = Vec::with_capacity(1);
    let mut current_metadata = None;

    for event in events {
        let Some(event) = event.as_array() else {
            continue;
        };
        match event.first().and_then(Value::as_i64) {
            Some(2) => {
                current_metadata = event.get(1).cloned();
            }
            Some(3) => {
                let Some(raw_url) = event.get(1).and_then(Value::as_str) else {
                    continue;
                };
                let Ok(file_url) = Url::parse(raw_url) else {
                    trace!(raw_url, "Skipping file event with unparseable URL");
                    continue;
                };
                let metadata = event.get(2).cloned().unwrap_or_default();
                results.push(GalleryDlEntry {
                    file_url,
                    metadata: merge_metadata(current_metadata.as_ref(), metadata),
                });
            }
            // Queue (6): an external URL for another extractor. The `--dump-json` pass emits these
            // unresolved; the `--resolve-json` pass follows them, so we only note it here.
            Some(6) => {
                if let Some(queued) = event.get(1).and_then(Value::as_str) {
                    trace!(url = queued, "Gallery-dl queued URL (resolved by the --resolve-json pass)");
                }
            }
            // gallery-dl reports extraction failures as a negative-type event carrying the error,
            // e.g. `[-1, {"error": "AbortExtraction", "message": "HTTP redirect to login page (…)"}]`.
            // Only surface it when nothing was parsed, so a late error can't discard already-found
            // files.
            Some(type_id) if type_id < 0 && results.is_empty() => {
                let message = event
                    .get(1)
                    .and_then(|details| {
                        details
                            .get("message")
                            .and_then(Value::as_str)
                            .or_else(|| details.get("error").and_then(Value::as_str))
                    })
                    .unwrap_or("Gallery-dl aborted extraction")
                    .to_owned();
                return Err(ParseJsonErrorKind::Extraction(message));
            }
            _ => {}
        }
    }

    Ok(results)
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

fn is_selected_by_range(index: i16, range: &Range) -> bool {
    index >= range.start && index <= range.count && (index - range.start) % range.step.abs() == 0
}

#[instrument(skip_all)]
pub async fn get_media_info(
    search: &str,
    gallery_dl_cfg: &GalleryDlConfig,
    playlist_range: &Range,
    timeout: u64,
    cookie_path: Option<&Path>,
) -> Result<Playlist, GetInfoErrorKind> {
    let request_url = Url::parse(search).map_err(io::Error::other)?;

    let mut entries = vec![];
    for resolve_urls in [false, true] {
        entries = get_playlist_entries(
            search,
            &request_url,
            gallery_dl_cfg,
            playlist_range,
            timeout,
            cookie_path,
            resolve_urls,
        )
        .await?;
        if !entries.is_empty() {
            break;
        }
        debug!(url = %search, resolve_urls, "Gallery-dl produced no photo entries");
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
    playlist_range: &Range,
    timeout: u64,
    cookie_path: Option<&Path>,
    resolve_urls: bool,
) -> Result<Vec<(crate::entities::MediaWithFormat, String)>, GetInfoErrorKind> {
    let GalleryDlOutput { stdout, logged_error } = run_gallery_dl_json(search, gallery_dl_cfg, timeout, cookie_path, resolve_urls).await?;
    let Some(stdout) = stdout else {
        // No output at all: surface a logged error if gallery-dl reported one.
        return logged_error.map_or(Ok(vec![]), |message| Err(GetInfoErrorKind::Extraction(message)));
    };

    let entries = parse_gallery_dl_json(&stdout)?
        .into_iter()
        .filter_map(|entry| match entry.into_raw_photo_info(request_url) {
            Ok(raw) => Some(raw),
            Err(reason) => {
                debug!(%reason, "Dropping gallery-dl entry");
                None
            }
        })
        .enumerate()
        .filter_map(|(idx, raw)| {
            let photo_index = i16::try_from(idx + 1).ok()?;
            is_selected_by_range(photo_index, playlist_range).then_some(RawPhotoInfo {
                playlist_index: photo_index,
                ..raw
            })
        })
        .map(RawPhotoInfo::into_playlist_entry)
        .collect::<Result<Vec<_>, _>>()
        .map_err(GetInfoErrorKind::from)?;

    // gallery-dl can log an extraction error yet exit 0 with no usable photos; prefer that message
    // over a generic "no photos" when we found nothing.
    if entries.is_empty() {
        if let Some(message) = logged_error {
            return Err(GetInfoErrorKind::Extraction(message));
        }
    }

    Ok(entries)
}

/// Output of a gallery-dl run: the raw JSON (`None` when empty) plus any error gallery-dl logged to
/// stderr while still exiting `0` (so the caller can surface it instead of a generic "no photos").
struct GalleryDlOutput {
    stdout: Option<Vec<u8>>,
    logged_error: Option<String>,
}

/// Extracts a gallery-dl `[category][error] <message>` line from stderr, returning the message.
/// gallery-dl logs extraction failures this way even when it exits `0` with empty output.
fn extract_logged_error(stderr: &str) -> Option<String> {
    stderr.lines().rev().find_map(|line| {
        let message = line.split_once("[error]")?.1.trim();
        (!message.is_empty()).then(|| message.to_owned())
    })
}

async fn run_gallery_dl_json(
    search: &str,
    gallery_dl_cfg: &GalleryDlConfig,
    timeout: u64,
    cookie_path: Option<&Path>,
    resolve_urls: bool,
) -> Result<GalleryDlOutput, GetInfoErrorKind> {
    let json_flag = if resolve_urls { "--resolve-json" } else { "--dump-json" };
    let mut args = vec![json_flag, "--simulate", "--no-input", "--config-ignore", "--no-colors"];

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
                return Err(process_exit_error("Gallery-dl", status, &stderr).into());
            }

            if !stderr.trim().is_empty() {
                warn!("{stderr}");
            }
            let stdout = (!stdout.iter().all(u8::is_ascii_whitespace)).then_some(stdout);

            Ok(GalleryDlOutput {
                stdout,
                logged_error: extract_logged_error(&stderr),
            })
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
                Err(process_exit_error("Gallery-dl", status, &stderr).into())
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
    fn surfaces_extraction_error_event() {
        let input = br#"[
            [-1, {"error": "AbortExtraction", "message": "HTTP redirect to login page (https://www.instagram.com/accounts/login/)"}]
        ]"#;

        let err = parse_gallery_dl_json(input).unwrap_err();

        match err {
            super::ParseJsonErrorKind::Extraction(message) => {
                assert!(message.contains("login page"), "unexpected message: {message}");
            }
            other => panic!("expected Extraction error, got {other:?}"),
        }
    }

    #[test]
    fn ignores_queue_message_without_files() {
        let input = br#"[
            [6, "https://www.tiktok.com/@user/video/123", {"category": "tiktok", "subcategory": "vmpost"}]
        ]"#;

        let entries = parse_gallery_dl_json(input).unwrap();

        assert!(entries.is_empty(), "queue messages should yield no direct entries");
    }

    #[test]
    fn extracts_logged_error_from_stderr() {
        let stderr = "[tiktok][error] https://www.tiktok.com/@u/video/1: Requested post not available";

        assert_eq!(
            super::extract_logged_error(stderr).as_deref(),
            Some("https://www.tiktok.com/@u/video/1: Requested post not available")
        );
    }

    #[test]
    fn ignores_non_error_stderr_lines() {
        assert!(super::extract_logged_error("[tiktok][warning] rate limited").is_none());
        assert!(super::extract_logged_error("").is_none());
    }

    #[test]
    fn ignores_metadata_only_event_array() {
        let input = br#"[
            [2, {"id": 42, "title": "Post", "url": "https://cdn.example/a.jpg"}]
        ]"#;

        let entries = parse_gallery_dl_json(input).unwrap();

        assert!(entries.is_empty());
    }

    #[test]
    fn rejects_root_metadata_object() {
        let input = br#"{
            "id": 42,
            "title": "Post",
            "media": [{"url": "https://cdn.example/a.jpg"}]
        }"#;

        let err = parse_gallery_dl_json(input).unwrap_err();

        assert!(matches!(err, super::ParseJsonErrorKind::UnexpectedRoot));
    }

    #[test]
    fn drops_video_file_events() {
        let input = br#"[
            [3, "https://cdn.example/video.mp4", {"extension": "mp4"}],
            [3, "https://cdn.example/song.mp3", {"extension": "mp3"}]
        ]"#;
        let request_url = Url::parse("https://www.tiktok.com/@user/photo/post").unwrap();

        let photos: Vec<_> = parse_gallery_dl_json(input)
            .unwrap()
            .into_iter()
            .filter_map(|entry| entry.into_raw_photo_info(&request_url).ok())
            .collect();

        assert!(photos.is_empty());
    }

    #[test]
    fn defaults_to_jpg_when_extension_is_missing() {
        let input = br#"[
            [3, "https://cdn.example/photo/abc123", {"id": "abc"}]
        ]"#;
        let request_url = Url::parse("https://example.com/post/1").unwrap();

        let raw = parse_gallery_dl_json(input)
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .into_raw_photo_info(&request_url)
            .unwrap();

        assert_eq!(raw.ext, "jpg");
    }

    #[test]
    fn accepts_modern_image_formats() {
        for ext in ["heic", "avif", "gif", "webp", "jxl"] {
            let raw_url = format!("https://cdn.example/photo.{ext}");
            let input = format!(r#"[[3, "{raw_url}", {{"extension": "{ext}"}}]]"#);
            let request_url = Url::parse("https://example.com/post/1").unwrap();

            let raw = parse_gallery_dl_json(input.as_bytes())
                .unwrap()
                .into_iter()
                .next()
                .unwrap()
                .into_raw_photo_info(&request_url)
                .unwrap();

            assert_eq!(raw.ext, ext);
        }
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
        let raw = entries.into_iter().next().unwrap().into_raw_photo_info(&request_url).unwrap();

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
