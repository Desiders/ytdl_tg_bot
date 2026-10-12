use crate::{
    config::GalleryDlConfig,
    entities::{GalleryDlEntry, Playlist, Range, RawPhotoInfo},
    utils::{process_exit_error, ProcessGroup},
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

#[derive(Debug)]
pub struct GalleryDlPost {
    pub id: Option<String>,
    pub text: String,
    pub author: Option<String>,
    pub author_handle: Option<String>,
    pub webpage_url: Url,
    pub media: Vec<GalleryDlEntry>,
}

#[instrument(skip_all)]
pub async fn get_posts(
    search: &str,
    gallery_dl_cfg: &GalleryDlConfig,
    timeout: u64,
    cookie_path: Option<&Path>,
) -> Result<Vec<GalleryDlPost>, GetInfoErrorKind> {
    let deadline = time::Instant::now() + Duration::from_secs(timeout);
    let request_url = Url::parse(search).map_err(io::Error::other)?;
    let options = [
        "-o",
        "text-tweets=true",
        "-o",
        "quoted=true",
        "-o",
        "conversations=false",
        "--post-range",
        "1-10",
    ];
    let GalleryDlOutput { stdout, logged_error } =
        run_gallery_dl_json(search, gallery_dl_cfg, timeout, cookie_path, true, &options).await?;
    let mut posts = stdout
        .as_deref()
        .map(|stdout| parse_gallery_dl_posts(stdout, &request_url))
        .transpose()?
        .unwrap_or_default();
    if posts.is_empty() {
        if let Some(message) = logged_error {
            return Err(GetInfoErrorKind::Extraction(message));
        }
    }
    let remaining = deadline.saturating_duration_since(time::Instant::now()).as_secs();
    if remaining > 0 && posts.len() < 10 && posts.first().is_some_and(|post| post.id.is_some()) {
        let options = [
            "-o",
            "text-tweets=true",
            "-o",
            "quoted=true",
            "-o",
            "conversations=true",
            "--post-range",
            "1-10",
        ];
        if let Ok(GalleryDlOutput { stdout: Some(stdout), .. }) =
            run_gallery_dl_json(search, gallery_dl_cfg, remaining, cookie_path, true, &options).await
        {
            if let Ok(context) = parse_gallery_dl_posts(&stdout, &request_url) {
                append_post_context(&mut posts, context);
            }
        }
    }
    let mut first = true;
    posts.retain(|post| {
        let keep = first || !post.text.trim().is_empty() || !post.media.is_empty();
        first = false;
        keep
    });
    Ok(posts)
}

fn append_post_context(posts: &mut Vec<GalleryDlPost>, context: Vec<GalleryDlPost>) {
    let Some(root_id) = posts.first().and_then(|post| post.id.as_ref()) else {
        return;
    };
    let Some(root_index) = context.iter().position(|post| post.id.as_ref() == Some(root_id)) else {
        return;
    };
    for post in context.into_iter().take(root_index) {
        if posts.len() == 10 {
            break;
        }
        let duplicate = post
            .id
            .as_ref()
            .is_some_and(|id| posts.iter().any(|existing| existing.id.as_ref() == Some(id)));
        if !duplicate && (!post.text.trim().is_empty() || !post.media.is_empty()) {
            posts.push(post);
        }
    }
}

fn post_from_metadata(metadata: Option<Value>, request_url: &Url) -> GalleryDlPost {
    let entry = GalleryDlEntry {
        file_url: request_url.clone(),
        metadata,
    };
    GalleryDlPost {
        id: entry.extract_id(),
        text: entry.extract_text().unwrap_or_default(),
        author: entry.extract_author_display_name(),
        author_handle: entry.extract_author_handle(),
        webpage_url: entry
            .extract_webpage_url()
            .filter(|url| matches!(url.scheme(), "http" | "https"))
            .unwrap_or_else(|| request_url.clone()),
        media: vec![],
    }
}

fn backfill_post_from_file(post: &mut GalleryDlPost, entry: &GalleryDlEntry, directory_metadata: Option<&Value>, request_url: &Url) {
    if post.id.is_none() {
        post.id = entry.extract_id();
    }
    if post.text.trim().is_empty() {
        post.text = entry.extract_text().unwrap_or_default();
    }
    if post.author.is_none() {
        post.author = entry.extract_author_display_name();
    }
    if post.author_handle.is_none() {
        post.author_handle = entry.extract_author_handle();
    }
    let directory_has_url = directory_metadata.is_some_and(|metadata| {
        GalleryDlEntry {
            file_url: request_url.clone(),
            metadata: Some(metadata.clone()),
        }
        .extract_webpage_url()
        .is_some_and(|url| matches!(url.scheme(), "http" | "https"))
    });
    if !directory_has_url && post.webpage_url == *request_url {
        if let Some(url) = entry.extract_webpage_url().filter(|url| matches!(url.scheme(), "http" | "https")) {
            post.webpage_url = url;
        }
    }
}

fn parse_gallery_dl_posts(input: &[u8], request_url: &Url) -> Result<Vec<GalleryDlPost>, ParseJsonErrorKind> {
    let Value::Array(events) = serde_json::from_slice(input)? else {
        return Err(ParseJsonErrorKind::UnexpectedRoot);
    };
    let mut posts: Vec<GalleryDlPost> = vec![];
    let mut current_metadata = None;
    for event in events {
        let Some(event) = event.as_array() else {
            continue;
        };
        match event.first().and_then(Value::as_i64) {
            Some(2) => {
                current_metadata = event.get(1).cloned();
                posts.push(post_from_metadata(current_metadata.clone(), request_url));
            }
            Some(3) => {
                if let Some(entry) = parse_file_event(event, current_metadata.as_ref()) {
                    if posts.is_empty() {
                        posts.push(post_from_metadata(entry.metadata.clone(), request_url));
                    }
                    if let Some(post) = posts.last_mut() {
                        backfill_post_from_file(post, &entry, current_metadata.as_ref(), request_url);
                        post.media.push(entry);
                    }
                }
            }
            Some(type_id) if type_id < 0 && posts.iter().all(|post| post.text.trim().is_empty() && post.media.is_empty()) => {
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
    if posts.iter().all(|post| post.text.trim().is_empty() && post.media.is_empty()) {
        posts.clear();
    } else {
        let mut first = true;
        posts.retain(|post| {
            let keep = first || post.id.is_some() || !post.text.trim().is_empty() || !post.media.is_empty();
            first = false;
            keep
        });
    }
    posts.truncate(10);
    Ok(posts)
}

fn parse_file_event(event: &[Value], metadata: Option<&Value>) -> Option<GalleryDlEntry> {
    let raw_url = event.get(1)?.as_str()?;
    let Ok(file_url) = Url::parse(raw_url) else {
        trace!(raw_url, "Skipping file event with unparseable URL");
        return None;
    };
    Some(GalleryDlEntry {
        file_url,
        metadata: merge_metadata(metadata, event.get(2).cloned().unwrap_or_default()),
    })
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
                if let Some(entry) = parse_file_event(event, current_metadata.as_ref()) {
                    results.push(entry);
                }
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
    let GalleryDlOutput { stdout, logged_error } =
        run_gallery_dl_json(search, gallery_dl_cfg, timeout, cookie_path, resolve_urls, &[]).await?;
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
            if !is_selected_by_range(photo_index, playlist_range) {
                return None;
            }
            Some(RawPhotoInfo {
                id: format!("{}_{photo_index}", raw.id),
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
    extra_args: &[&str],
) -> Result<GalleryDlOutput, GetInfoErrorKind> {
    let json_flag = if resolve_urls { "--resolve-json" } else { "--dump-json" };
    let mut args = vec![json_flag, "--simulate", "--no-input", "--config-ignore", "--no-colors"];
    args.extend_from_slice(extra_args);

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
        .process_group(0)
        .spawn()?;
    let _process_group = ProcessGroup::new(&child);

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
    fn uses_populated_native_text_after_empty_content() {
        let request_url = Url::parse("https://example.invalid/post/requested").unwrap();
        let input = br#"[
            [2, {"content": "", "description": "Description text"}],
            [2, {"content": "   ", "caption": "Caption text"}],
            [2, {"content": "", "description": "", "caption": "", "text": "Text field"}]
        ]"#;

        let posts = super::parse_gallery_dl_posts(input, &request_url).unwrap();
        assert_eq!(
            posts.iter().map(|post| post.text.as_str()).collect::<Vec<_>>(),
            ["Description text", "Caption text", "Text field"]
        );
    }

    #[test]
    fn backfills_missing_post_metadata_from_url_event_without_replacing_directory_values() {
        let request_url = Url::parse("https://example.invalid/post/requested").unwrap();
        let input = br#"[
            [2, {"content": "", "author": "Directory Author", "post_url": "https://example.invalid/post/directory"}],
            [3, "https://cdn.example.invalid/photo.jpg", {"id": "native-id", "description": "File description", "author": "File Author", "username": "file_handle", "page_url": "https://example.invalid/post/file"}],
            [2, {"id": "second"}],
            [3, "https://cdn.example.invalid/second.jpg", {"caption": "Second file caption", "fullname": "Second Author", "handle": "second_handle", "page_url": "https://example.invalid/post/second"}]
        ]"#;

        let posts = super::parse_gallery_dl_posts(input, &request_url).unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].id.as_deref(), Some("native-id"));
        assert_eq!(posts[0].text, "File description");
        assert_eq!(posts[0].author.as_deref(), Some("Directory Author"));
        assert_eq!(posts[0].author_handle.as_deref(), Some("file_handle"));
        assert_eq!(posts[0].webpage_url.as_str(), "https://example.invalid/post/directory");
        assert_eq!(posts[1].text, "Second file caption");
        assert_eq!(posts[1].author.as_deref(), Some("Second Author"));
        assert_eq!(posts[1].author_handle.as_deref(), Some("second_handle"));
        assert_eq!(posts[1].webpage_url.as_str(), "https://example.invalid/post/second");
    }

    #[test]
    fn preserves_text_only_posts_and_groups_mixed_media_and_quotes() {
        let input = br#"[
            [2, {"content": "Text only <unchanged>", "author": "Synthetic Author", "user": {"name": "root_handle", "nick": "Root Author"}}],
            [2, {"content": "Mixed media", "author": {"nick": "Display Name", "name": "synthetic_handle"}, "post_url": "javascript:invalid"}],
            [3, "https://cdn.example.invalid/photo.jpg", {"type": "photo", "width": 640}],
            [3, "https://cdn.example.invalid/video.mp4", {"type": "video", "content": "File metadata override"}],
            [2, {"description": "Quoted text only", "page_url": "https://example.invalid/post/quoted", "user": {"display_name": "Quote Author", "username": "quote_handle"}, "fullname": "Unused Top-level Name", "username": "unused_top_level_handle"}],
            [2, {"description": "Native top-level author", "fullname": "Top-level Author", "username": "top_level_handle"}],
            [2, {"description": "Native author aliases", "user": {"display_name": " ", "username": " "}, "fullname": " ", "full_name": "Alias Author", "username": " ", "handle": "alias_handle"}],
            [2, {}]
        ]"#;
        let request_url = Url::parse("https://example.invalid/post/requested").unwrap();
        let posts = super::parse_gallery_dl_posts(input, &request_url).unwrap();

        assert_eq!(posts.len(), 5);
        assert_eq!(posts[0].text, "Text only <unchanged>");
        assert_eq!(posts[0].author.as_deref(), Some("Synthetic Author"));
        assert!(posts[0].media.is_empty());
        assert_eq!(posts[1].text, "Mixed media");
        assert_eq!(posts[1].author.as_deref(), Some("Display Name"));
        assert_eq!(posts[1].author_handle.as_deref(), Some("synthetic_handle"));
        assert_eq!(posts[1].webpage_url, request_url);
        assert_eq!(posts[1].media.len(), 2);
        assert_eq!(posts[1].media[0].file_url.as_str(), "https://cdn.example.invalid/photo.jpg");
        assert_eq!(posts[1].media[0].extract_text().as_deref(), Some("Mixed media"));
        assert_eq!(posts[1].media[1].file_url.as_str(), "https://cdn.example.invalid/video.mp4");
        assert_eq!(posts[1].media[1].extract_text().as_deref(), Some("File metadata override"));
        assert_eq!(posts[2].text, "Quoted text only");
        assert_eq!(posts[2].webpage_url.as_str(), "https://example.invalid/post/quoted");
        assert_eq!(posts[2].author.as_deref(), Some("Quote Author"));
        assert_eq!(posts[2].author_handle.as_deref(), Some("quote_handle"));
        assert!(posts[2].media.is_empty());
        assert_eq!(posts[3].text, "Native top-level author");
        assert_eq!(posts[3].author.as_deref(), Some("Top-level Author"));
        assert_eq!(posts[3].author_handle.as_deref(), Some("top_level_handle"));
        assert_eq!(posts[4].author.as_deref(), Some("Alias Author"));
        assert_eq!(posts[4].author_handle.as_deref(), Some("alias_handle"));

        let primary = br#"[
            [2, {"id": "synthetic-root"}],
            [2, {"id": "synthetic-quote", "content": "Primary quoted text"}]
        ]"#;
        let mut posts = super::parse_gallery_dl_posts(primary, &request_url).unwrap();
        assert!(posts[0].text.is_empty());
        let context = br#"[
            [2, {"id": "synthetic-parent", "content": "Preceding context"}],
            [2, {"id": "synthetic-quote", "content": "Duplicate context"}],
            [2, {"id": "synthetic-root"}],
            [2, {"id": "synthetic-unrelated", "content": "Following context"}]
        ]"#;
        let context = super::parse_gallery_dl_posts(context, &request_url).unwrap();
        assert_eq!(context[2].id.as_deref(), Some("synthetic-root"));
        assert!(context[2].text.is_empty());
        super::append_post_context(&mut posts, context);
        assert_eq!(
            posts.iter().map(|post| post.id.as_deref()).collect::<Vec<_>>(),
            [Some("synthetic-root"), Some("synthetic-quote"), Some("synthetic-parent")]
        );
        assert!(posts[0].text.is_empty());
        assert_eq!(posts[1].text, "Primary quoted text");
        assert_eq!(posts[2].text, "Preceding context");
        let unrelated = br#"[[2, {"id": "synthetic-other", "content": "Unmatched context"}]]"#;
        super::append_post_context(&mut posts, super::parse_gallery_dl_posts(unrelated, &request_url).unwrap());
        assert_eq!(posts.len(), 3);
        let empty = br#"[[2, {"id": "synthetic-root"}], [2, {}]]"#;
        assert!(super::parse_gallery_dl_posts(empty, &request_url).unwrap().is_empty());
    }

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
