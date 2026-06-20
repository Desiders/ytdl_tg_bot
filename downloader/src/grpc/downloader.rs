use std::{
    ffi::OsStr,
    fs,
    pin::Pin,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Instant,
};

use proto::downloader::{
    download_chunk::Payload, downloader_server::Downloader, DownloadChunk, DownloadMeta, DownloadRequest, MediaEntry, MediaFormatEntry,
    MediaInfoRequest, MediaInfoResponse, Section,
};
use tempfile::TempDir;
use tokio::{
    fs::File,
    io::AsyncReadExt as _,
    sync::{
        mpsc::{self, UnboundedSender},
        Semaphore,
    },
};
use tokio_stream::{wrappers::UnboundedReceiverStream, Stream};
use tonic::{Code, Request, Response, Status};
use tracing::{debug, error, info, warn};
use url::Url;

use crate::{
    config::{GalleryDlConfig, YtDlpConfig, YtPotProviderConfig},
    entities::{Cookies, Language, Media, MediaFormat, MediaWithFormat, Playlist, Range, RawPhotoInfo, Sections},
    services::{
        domain_replacer::{DomainReplacer, MediaKind},
        download_and_convert, embed_thumbnail,
        gallery_dl::{self, GetInfoErrorKind},
        probe_video, remux_copy,
        snapsave::{ResolvedMedia, SnapsaveOutcome, SnapsaveResolver},
        ytdl::{self, FormatStrategy},
    },
};

const GET_INFO_TIMEOUT_SECS: u64 = 180;
const DOWNLOAD_TIMEOUT_SECS: u64 = 420;
const AUDIO_EXT: &str = "m4a";
const THUMBNAIL_TIMEOUT_SECS: u64 = 5;
const FFMPEG_PATH: &str = "/usr/bin/ffmpeg";
const FFPROBE_PATH: &str = "/usr/bin/ffprobe";
const STREAM_CHUNK_SIZE: usize = 256 * 1024;

type DownloadStream = Pin<Box<dyn Stream<Item = Result<DownloadChunk, Status>> + Send + 'static>>;

pub struct DownloaderService {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub gallery_dl_cfg: Arc<GalleryDlConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub domain_replacer: Arc<DomainReplacer>,
    pub snapsave: Arc<SnapsaveResolver>,
    pub cookies: Arc<Cookies>,
    pub active_downloads: Arc<AtomicU32>,
    pub semaphore: Arc<Semaphore>,
}

#[tonic::async_trait]
impl Downloader for DownloaderService {
    type DownloadMediaStream = DownloadStream;

    async fn get_media_info(&self, request: Request<MediaInfoRequest>) -> Result<Response<MediaInfoResponse>, Status> {
        let started_at = Instant::now();
        let request = request.into_inner();
        let url = parse_url(&request.url)?;
        let playlist_range = parse_range(request.playlist_range)?;
        let audio_language = parse_language(&request.audio_language);
        let allow_playlist = !playlist_range.is_single_element();
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());
        let has_cookie = cookie.is_some();
        let effective_max_file_size = resolve_max_file_size(request.max_file_size, self.yt_dlp_cfg.max_file_size);

        let kind = match request.media_type.as_str() {
            "audio" => MediaKind::Audio,
            "photo" => MediaKind::Photo,
            _ => MediaKind::Video,
        };
        // Source URLs to fetch info from. Instagram/Facebook with no cookie resolve to direct CDN
        // URLs via snapsave (showing the original URL); otherwise the deterministic cookie-aware
        // domain replacement (vxinstagram fallback). `original` is set when we must override the
        // reported `webpage_url` back to the post URL.
        let snapsave_outcome = if !has_cookie && self.snapsave.is_supported(&url) {
            self.snapsave.resolve(&url, kind).await
        } else {
            SnapsaveOutcome::Unavailable
        };
        let (source_items, snapsave_url) = match snapsave_outcome {
            SnapsaveOutcome::Resolved(items) => {
                let items = select_by_range(items, &playlist_range);
                if items.is_empty() {
                    return Err(Status::not_found("Requested items are out of range"));
                }
                (items, Some(url.clone()))
            }
            SnapsaveOutcome::WrongKind => return Err(Status::not_found("No media of the requested type")),
            SnapsaveOutcome::Unavailable => {
                let source_url = if has_cookie {
                    url.clone()
                } else {
                    self.domain_replacer.replace_url(&url, kind).unwrap_or_else(|| url.clone())
                };
                (
                    vec![ResolvedMedia {
                        url: source_url,
                        thumbnail: None,
                    }],
                    None,
                )
            }
        };

        info!(
            url = %url,
            source_count = source_items.len(),
            via_snapsave = snapsave_url.is_some(),
            media_type = %request.media_type,
            audio_language = %request.audio_language,
            allow_playlist,
            has_cookie,
            max_file_size = effective_max_file_size,
            "Fetching media info"
        );

        let mut playlist = Playlist { inner: vec![] };
        for item in &source_items {
            let source_url = &item.url;
            let source_cookie = if snapsave_url.is_none() && source_url.as_str() == url.as_str() {
                cookie.as_deref()
            } else {
                None
            };
            let part = if request.media_type == "photo" {
                if snapsave_url.is_some() {
                    synthesize_photo(item, &url)?
                } else {
                    gallery_dl::get_media_info(
                        source_url.as_str(),
                        self.gallery_dl_cfg.as_ref(),
                        &playlist_range,
                        GET_INFO_TIMEOUT_SECS,
                        source_cookie,
                    )
                    .await
                    .map_err(|err| match err {
                        GetInfoErrorKind::EmptyEntries => Status::not_found(err.to_string()),
                        err => Status::internal(err.to_string()),
                    })?
                }
            } else if snapsave_url.is_some() && request.media_type == "video" {
                synthesize_video(item, &url)?
            } else {
                let format_strategy = parse_format_strategy(&request.media_type, &request.audio_language)?;
                let part = ytdl::get_media_info(
                    source_url.as_str(),
                    &format_strategy,
                    &audio_language,
                    self.yt_dlp_cfg.as_ref(),
                    &self.yt_pot_provider_cfg.url,
                    &playlist_range,
                    allow_playlist,
                    GET_INFO_TIMEOUT_SECS,
                    source_cookie,
                )
                .await?;
                reject_active_livestreams(&part)?;
                part
            };
            for (mut media, mut formats) in part.inner {
                if let Some(original) = &snapsave_url {
                    media.webpage_url = original.clone();
                    // Name from the original post (e.g. reel shortcode), not the CDN path ("v2"); a
                    // bare CDN id also collides across posts in the per-item cache.
                    let base = snapsave_name(original);
                    let name = if source_items.len() > 1 {
                        format!("{base}_{}", playlist.inner.len() + 1)
                    } else {
                        base
                    };
                    media.id = name.clone();
                    media.title = Some(name);
                    // yt-dlp's generic info gives the tokenized CDN URL no real ext and no thumbnail.
                    // Patch the `--load-info-json` blob (which the separate download request reads) so
                    // the file gets a valid container (else metadata postprocessing fails) and a poster.
                    for (format, raw) in &mut formats {
                        if format.ext == "unknown_video" {
                            format.ext = "mp4".to_owned();
                        }
                        *raw = patch_info_json(raw, &format.ext, item.thumbnail.as_ref());
                    }
                }
                playlist.inner.push((media, formats));
            }
        }
        // Reindex a snapsave carousel so each item keeps a distinct position.
        if snapsave_url.is_some() && playlist.inner.len() > 1 {
            for (index, (media, _)) in playlist.inner.iter_mut().enumerate() {
                media.playlist_index = i16::try_from(index + 1).unwrap_or(media.playlist_index);
            }
        }

        let entries_count = playlist.inner.len();
        info!(
            url = %url,
            media_type = %request.media_type,
            entries_count,
            elapsed_ms = started_at.elapsed().as_millis(),
            "Fetched media info"
        );

        Ok(Response::new(map_playlist_response(playlist, effective_max_file_size)?))
    }

    async fn download_media(&self, request: Request<DownloadRequest>) -> Result<Response<Self::DownloadMediaStream>, Status> {
        let request = request.into_inner();
        let request_url = request.url.clone();
        let request_format_id = request.format_id.clone();
        let request_media_type = request.media_type.clone();
        let permit = self.semaphore.clone().try_acquire_owned().map_err(|_| {
            warn!(
                url = %request_url,
                format_id = %request_format_id,
                media_type = %request_media_type,
                "Rejected download request because node is at capacity"
            );
            Status::resource_exhausted("Node is at capacity")
        })?;
        let active_downloads = self.active_downloads.fetch_add(1, Ordering::Relaxed) + 1;
        info!(
            url = %request_url,
            format_id = %request_format_id,
            media_type = %request_media_type,
            active_downloads,
            "Accepted download request"
        );

        let guard = DownloadGuard {
            active_downloads: self.active_downloads.clone(),
            _permit: permit,
        };
        let yt_dlp_cfg = self.yt_dlp_cfg.clone();
        let gallery_dl_cfg = self.gallery_dl_cfg.clone();
        let yt_pot_provider_cfg = self.yt_pot_provider_cfg.clone();
        let domain_replacer = self.domain_replacer.clone();
        let cookies = self.cookies.clone();

        let (tx, rx) = mpsc::unbounded_channel();
        let error_tx = tx.clone();
        tokio::spawn(async move {
            let started_at = Instant::now();
            if let Err(status) = stream_download(
                request,
                yt_dlp_cfg,
                gallery_dl_cfg,
                yt_pot_provider_cfg,
                domain_replacer,
                cookies,
                tx,
            )
            .await
            {
                if status.code() == Code::Cancelled {
                    info!(
                        url = %request_url,
                        format_id = %request_format_id,
                        media_type = %request_media_type,
                        elapsed_ms = started_at.elapsed().as_millis(),
                        "Download stream cancelled by client"
                    );
                } else {
                    error!(
                        url = %request_url,
                        format_id = %request_format_id,
                        media_type = %request_media_type,
                        elapsed_ms = started_at.elapsed().as_millis(),
                        %status,
                        "Download stream failed"
                    );
                    let _ = send_status(&error_tx, status).await;
                }
            } else {
                info!(
                    url = %request_url,
                    format_id = %request_format_id,
                    media_type = %request_media_type,
                    elapsed_ms = started_at.elapsed().as_millis(),
                    "Download stream finished"
                );
            }
            drop(guard);
        });

        Ok(Response::new(Box::pin(UnboundedReceiverStream::new(rx))))
    }
}

struct DownloadGuard {
    active_downloads: Arc<AtomicU32>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        self.active_downloads.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn stream_download(
    request: DownloadRequest,
    yt_dlp_cfg: Arc<YtDlpConfig>,
    gallery_dl_cfg: Arc<GalleryDlConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    domain_replacer: Arc<DomainReplacer>,
    cookies: Arc<Cookies>,
    tx: UnboundedSender<Result<DownloadChunk, Status>>,
) -> Result<(), Status> {
    let url = parse_url(&request.url)?;
    let section = parse_section(request.section);
    let effective_max_file_size = resolve_max_file_size(request.max_file_size, yt_dlp_cfg.max_file_size);
    let host = url.host();
    let cookie = cookies.get_path_by_optional_host(host.as_ref());

    if request.media_type == "photo" {
        // Mirror the deterministic switching from get-info so the download fetches from the same
        // host the `direct_url` filter was built against.
        let photo_url = if cookie.is_some() {
            url.clone()
        } else {
            domain_replacer.replace_url(&url, MediaKind::Photo).unwrap_or_else(|| url.clone())
        };
        // The original URL uses its cookie; the proxy is a cookie-free frontend.
        let photo_cookie = if photo_url.as_str() == url.as_str() { cookie } else { None };
        return stream_photo_download(request, photo_url, effective_max_file_size, gallery_dl_cfg, photo_cookie, tx).await;
    }

    let media_with_format: MediaWithFormat =
        serde_json::from_str(&request.raw_info_json).map_err(|err| Status::invalid_argument(format!("Invalid info file error: {err}")))?;
    let media = Media::from(media_with_format.clone());
    if media_with_format.direct_fetch {
        let direct_url = media_with_format
            .direct_url
            .clone()
            .ok_or_else(|| Status::invalid_argument("Direct-fetch video is missing its URL"))?;
        let format = MediaFormat::from(media_with_format);
        return stream_direct_video(&url, &media, &format, &direct_url, effective_max_file_size, tx).await;
    }
    let strategy = parse_format_strategy(&request.media_type, &request.audio_ext)?;
    // Only a single pre-muxed HTTP(S) format is streamable: it's already a faststart MP4, so it
    // plays and seeks on the client. HLS/DASH and merges can't be piped as a seekable file, so they
    // take the download path (which produces a seekable faststart MP4).
    let can_stream = matches!(strategy, FormatStrategy::VideoAndAudio)
        && section.is_none()
        && media_with_format.is_progressive_streamable()
        && media_with_format
            .filesize_approx
            .is_some_and(|size| size <= effective_max_file_size);
    let format = MediaFormat::from(media_with_format);
    let ext = strategy.output_ext(format.ext.as_str()).to_owned();

    info!(
        url = %url,
        format_id = %request.format_id,
        media_type = ?strategy,
        has_cookie = cookie.is_some(),
        has_section = section.is_some(),
        section_start = section.as_ref().and_then(|section| section.start),
        section_end = section.as_ref().and_then(|section| section.end),
        max_file_size = effective_max_file_size,
        can_stream,
        "Starting media download"
    );

    if can_stream {
        return stream_piped_download(
            &url,
            &request.format_id,
            &request.raw_info_json,
            &media,
            &format,
            ext,
            effective_max_file_size,
            yt_dlp_cfg.as_ref(),
            yt_pot_provider_cfg.as_ref(),
            cookie.as_deref(),
            tx,
        )
        .await;
    }

    let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(|err| Status::internal(format!("Temp dir error: {err}")))?;
    let output_dir_path = temp_dir.path();
    let info_file_path = output_dir_path.join("media.info.json");
    fs::write(&info_file_path, &request.raw_info_json).map_err(|err| Status::internal(format!("Info file error: {err}")))?;

    let media_file_path = output_dir_path.join(format!("media.{ext}"));
    let thumb_file_path = output_dir_path.join("media.jpg");
    let thumb_urls = media.get_thumb_urls(format.aspect_ratio_kind());

    debug!(
        url = %url,
        thumbnail_candidate_count = thumb_urls.len(),
        path = %thumb_file_path.display(),
        "Starting thumbnail download"
    );
    let thumbnail_downloaded = download_thumbnail(&thumb_urls, &thumb_file_path).await;
    debug!(
        url = %url,
        thumbnail_downloaded,
        path = %thumb_file_path.display(),
        "Thumbnail download step finished"
    );

    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<String>();
    let mut download_future = Box::pin(ytdl::download_media(
        strategy,
        &request.format_id,
        section.as_ref(),
        effective_max_file_size,
        output_dir_path,
        &info_file_path,
        yt_dlp_cfg.as_ref(),
        &yt_pot_provider_cfg.url,
        DOWNLOAD_TIMEOUT_SECS,
        cookie.as_deref(),
        Some(&progress_tx),
    ));

    // Stream progress in real time while the download runs, but hold back `Meta` until the
    // download has succeeded and the output file is validated. The bot starts its Telegram
    // upload the moment it sees `Meta`, so emitting it early (e.g. on the first progress line)
    // would turn a later download failure into a corrupt, half-streamed upload instead of a
    // clean error the bot can retry with another format. Racing `tx.closed()` lets a client
    // disconnect kill yt-dlp via `kill_on_drop` rather than wasting CPU/disk/bandwidth.
    loop {
        tokio::select! {
            Some(progress) = progress_rx.recv() => {
                send_chunk(&tx, DownloadChunk { payload: Some(Payload::Progress(progress)) }).await?;
            }
            res = &mut download_future => {
                res.map_err(download_error_status)?;
                break;
            }
            () = tx.closed() => {
                return Err(Status::cancelled("Client disconnected"));
            }
        }
    }
    drop(download_future);
    drop(progress_tx);

    let media_file_path = resolve_media_file_path(output_dir_path, &media_file_path).await?;
    if thumbnail_downloaded {
        try_embed_thumbnail(&url, &media_file_path, &thumb_file_path).await;
    }
    let file_size = validate_download_file(&url, &media_file_path, effective_max_file_size).await?;

    send_chunk(
        &tx,
        DownloadChunk {
            payload: Some(Payload::Meta(DownloadMeta {
                ext,
                width: format.width,
                height: format.height,
                duration: resolve_download_duration(media.duration, section.as_ref()),
                has_thumbnail: thumbnail_downloaded,
            })),
        },
    )
    .await?;

    if thumbnail_downloaded {
        debug!(url = %url, path = %thumb_file_path.display(), "Starting thumbnail stream to client");
        stream_thumbnail_file(&thumb_file_path, &tx).await?;
        debug!(url = %url, path = %thumb_file_path.display(), "Finished thumbnail stream to client");
    }

    stream_media_to_client(&url, &media_file_path, file_size, &tx).await?;
    drop(temp_dir);
    Ok(())
}

// Stream a single progressive format with no intermediate file: yt-dlp writes the media to stdout
// and we forward each chunk to the client as it arrives. `Meta` is held back until the first byte
// of media is read, so a failure during format resolution / connection still surfaces as a clean
// error the bot can fail over to another format. A failure after the first byte aborts the
// already-started upload — the price of not downloading first.
#[allow(clippy::too_many_arguments)]
async fn stream_piped_download(
    url: &Url,
    format_id: &str,
    raw_info_json: &str,
    media: &Media,
    format: &MediaFormat,
    ext: String,
    effective_max_file_size: u64,
    yt_dlp_cfg: &YtDlpConfig,
    yt_pot_provider_cfg: &YtPotProviderConfig,
    cookie: Option<&std::path::Path>,
    tx: UnboundedSender<Result<DownloadChunk, Status>>,
) -> Result<(), Status> {
    let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(|err| Status::internal(format!("Temp dir error: {err}")))?;
    let output_dir_path = temp_dir.path();
    let info_file_path = output_dir_path.join("media.info.json");
    fs::write(&info_file_path, raw_info_json).map_err(|err| Status::internal(format!("Info file error: {err}")))?;

    let thumb_file_path = output_dir_path.join("media.jpg");
    let thumb_urls = media.get_thumb_urls(format.aspect_ratio_kind());
    debug!(
        url = %url,
        thumbnail_candidate_count = thumb_urls.len(),
        "Starting thumbnail download for streamed media"
    );
    // Fetch the thumbnail concurrently with yt-dlp's connection/first-byte latency rather than
    // serializing it ahead of the download; it only has to be ready by the time the first media
    // chunk arrives (when `Meta` and the thumbnail stream are emitted).
    let mut thumb_task = Some(tokio::spawn({
        let thumb_file_path = thumb_file_path.clone();
        async move { download_thumbnail(&thumb_urls, &thumb_file_path).await }
    }));
    let duration = resolve_download_duration(media.duration, None);

    let (item_tx, mut item_rx) = mpsc::unbounded_channel::<ytdl::StreamItem>();
    let mut download_future = Box::pin(ytdl::stream_media(
        format_id,
        effective_max_file_size,
        &info_file_path,
        yt_dlp_cfg,
        &yt_pot_provider_cfg.url,
        DOWNLOAD_TIMEOUT_SECS,
        cookie,
        item_tx,
    ));

    let mut meta_sent = false;
    let mut download_done = false;
    let mut items_drained = false;
    let mut total_bytes = 0_u64;

    loop {
        tokio::select! {
            biased;
            () = tx.closed() => return Err(Status::cancelled("Client disconnected")),
            res = &mut download_future, if !download_done => {
                res.map_err(download_error_status)?;
                download_done = true;
                if items_drained {
                    break;
                }
            }
            item = item_rx.recv(), if !items_drained => {
                match item {
                    Some(ytdl::StreamItem::Progress(progress)) => {
                        send_chunk(&tx, DownloadChunk { payload: Some(Payload::Progress(progress)) }).await?;
                    }
                    Some(ytdl::StreamItem::Data(data)) => {
                        total_bytes += data.len() as u64;
                        if total_bytes > effective_max_file_size {
                            warn!(
                                url = %url,
                                total_bytes,
                                max_file_size = effective_max_file_size,
                                "Streamed media exceeds max file size"
                            );
                            return Err(Status::invalid_argument("File exceeds max file size"));
                        }
                        if !meta_sent {
                            let thumbnail_downloaded = match thumb_task.take() {
                                Some(handle) => handle.await.unwrap_or(false),
                                None => false,
                            };
                            send_chunk(
                                &tx,
                                DownloadChunk {
                                    payload: Some(Payload::Meta(DownloadMeta {
                                        ext: ext.clone(),
                                        width: format.width,
                                        height: format.height,
                                        duration,
                                        has_thumbnail: thumbnail_downloaded,
                                    })),
                                },
                            )
                            .await?;
                            if thumbnail_downloaded {
                                stream_thumbnail_file(&thumb_file_path, &tx).await?;
                            }
                            meta_sent = true;
                        }
                        send_chunk(&tx, DownloadChunk { payload: Some(Payload::Data(data)) }).await?;
                    }
                    None => {
                        items_drained = true;
                        if download_done {
                            break;
                        }
                    }
                }
            }
        }
    }

    if !meta_sent {
        if let Some(handle) = thumb_task.take() {
            handle.abort();
        }
        warn!(url = %url, "Streamed media produced no data");
        return Err(Status::internal("Downloaded file is empty"));
    }

    info!(url = %url, bytes_streamed = total_bytes, "Finished streaming downloaded media");
    drop(temp_dir);
    Ok(())
}

async fn stream_photo_download(
    request: DownloadRequest,
    url: Url,
    effective_max_file_size: u64,
    gallery_dl_cfg: Arc<GalleryDlConfig>,
    cookie: Option<std::path::PathBuf>,
    tx: UnboundedSender<Result<DownloadChunk, Status>>,
) -> Result<(), Status> {
    let raw_info: RawPhotoInfo =
        serde_json::from_str(&request.raw_info_json).map_err(|err| Status::invalid_argument(format!("Invalid info file error: {err}")))?;

    info!(
        url = %url,
        format_id = %request.format_id,
        media_type = "photo",
        has_cookie = cookie.is_some(),
        max_file_size = effective_max_file_size,
        "Starting media download"
    );

    let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(|err| Status::internal(format!("Temp dir error: {err}")))?;
    let output_dir_path = temp_dir.path();
    let media_file_path = output_dir_path.join(format!("media.{}", raw_info.ext));

    if raw_info.direct {
        download_and_convert(raw_info.direct_url.as_str(), &media_file_path, FFMPEG_PATH, DOWNLOAD_TIMEOUT_SECS)
            .await
            .map_err(|err| Status::internal(err.to_string()))?;
    } else {
        gallery_dl::download_media(
            url.as_str(),
            &raw_info,
            effective_max_file_size,
            output_dir_path,
            gallery_dl_cfg.as_ref(),
            DOWNLOAD_TIMEOUT_SECS,
            cookie.as_deref(),
        )
        .await
        .map_err(|err| Status::internal(err.to_string()))?;
    }

    let media_file_path = resolve_media_file_path(output_dir_path, &media_file_path).await?;
    let file_size = validate_download_file(&url, &media_file_path, effective_max_file_size).await?;

    send_chunk(
        &tx,
        DownloadChunk {
            payload: Some(Payload::Meta(DownloadMeta {
                ext: raw_info.ext,
                width: raw_info.width,
                height: raw_info.height,
                duration: None,
                has_thumbnail: false,
            })),
        },
    )
    .await?;

    stream_media_to_client(&url, &media_file_path, file_size, &tx).await?;
    drop(temp_dir);
    Ok(())
}

async fn stream_direct_video(
    url: &Url,
    media: &Media,
    format: &MediaFormat,
    direct_url: &Url,
    effective_max_file_size: u64,
    tx: UnboundedSender<Result<DownloadChunk, Status>>,
) -> Result<(), Status> {
    info!(url = %url, direct_url = %direct_url, media_type = "video", "Starting direct media download");

    let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(|err| Status::internal(format!("Temp dir error: {err}")))?;
    let output_dir_path = temp_dir.path();
    let media_file_path = output_dir_path.join("media.mp4");
    let thumb_file_path = output_dir_path.join("media.jpg");
    let thumb_urls = media.get_thumb_urls(format.aspect_ratio_kind());

    let thumbnail_downloaded = download_thumbnail(&thumb_urls, &thumb_file_path).await;

    // Racing `tx.closed()` lets a client disconnect kill ffmpeg via `kill_on_drop`.
    let mut remux_future = Box::pin(remux_copy(
        direct_url.as_str(),
        &media_file_path,
        FFMPEG_PATH,
        DOWNLOAD_TIMEOUT_SECS,
    ));
    tokio::select! {
        res = &mut remux_future => res.map_err(|err| Status::internal(err.to_string()))?,
        () = tx.closed() => return Err(Status::cancelled("Client disconnected")),
    }
    drop(remux_future);

    let media_file_path = resolve_media_file_path(output_dir_path, &media_file_path).await?;
    if thumbnail_downloaded {
        try_embed_thumbnail(url, &media_file_path, &thumb_file_path).await;
    }
    let file_size = validate_download_file(url, &media_file_path, effective_max_file_size).await?;

    let probed = probe_video(&media_file_path, FFPROBE_PATH, THUMBNAIL_TIMEOUT_SECS).await;
    send_chunk(
        &tx,
        DownloadChunk {
            payload: Some(Payload::Meta(DownloadMeta {
                ext: "mp4".to_owned(),
                width: probed.width.or(format.width),
                height: probed.height.or(format.height),
                duration: resolve_download_duration(probed.duration.or(media.duration), None),
                has_thumbnail: thumbnail_downloaded,
            })),
        },
    )
    .await?;

    if thumbnail_downloaded {
        stream_thumbnail_file(&thumb_file_path, &tx).await?;
    }
    stream_media_to_client(url, &media_file_path, file_size, &tx).await?;
    drop(temp_dir);
    Ok(())
}

/// Embed `thumb_file_path` into `media_file_path`. Best-effort — failures are
/// logged but not surfaced to the caller (the media file is still usable).
async fn try_embed_thumbnail(url: &Url, media_file_path: &std::path::Path, thumb_file_path: &std::path::Path) {
    let embed_started_at = Instant::now();
    let media_size_before_embed = file_size(media_file_path).await;
    let thumbnail_size = file_size(thumb_file_path).await;
    debug!(
        url = %url,
        media_path = %media_file_path.display(),
        thumbnail_path = %thumb_file_path.display(),
        ?media_size_before_embed,
        ?thumbnail_size,
        "Starting thumbnail embed"
    );
    match embed_thumbnail(media_file_path, thumb_file_path).await {
        Ok(()) => {
            let media_size_after_embed = file_size(media_file_path).await;
            debug!(
                url = %url,
                media_path = %media_file_path.display(),
                ?media_size_after_embed,
                elapsed_ms = embed_started_at.elapsed().as_millis(),
                "Thumbnail embed finished"
            );
        }
        Err(err) => {
            error!(
                url = %url,
                media_path = %media_file_path.display(),
                thumbnail_path = %thumb_file_path.display(),
                elapsed_ms = embed_started_at.elapsed().as_millis(),
                %err,
                "Thumbnail embed failed"
            );
        }
    }
}

/// Validate the downloaded file before its `Meta` is sent: it must be non-empty
/// (yt-dlp/gallery-dl can exit 0 yet leave an empty file) and fit the size budget.
/// Returns the validated size on success.
async fn validate_download_file(url: &Url, media_file_path: &std::path::Path, effective_max_file_size: u64) -> Result<u64, Status> {
    let metadata = tokio::fs::metadata(media_file_path)
        .await
        .map_err(|err| Status::internal(format!("Metadata error: {err}")))?;
    let file_size = metadata.len();
    if file_size == 0 {
        warn!(url = %url, path = %media_file_path.display(), "Downloaded file is empty");
        return Err(Status::internal("Downloaded file is empty"));
    }
    if file_size > effective_max_file_size {
        warn!(
            url = %url,
            file_size,
            max_file_size = effective_max_file_size,
            "Downloaded file exceeds max file size"
        );
        return Err(Status::invalid_argument("File exceeds max file size"));
    }
    Ok(file_size)
}

/// Stream a validated media file to the gRPC client as `Data` chunks.
async fn stream_media_to_client(
    url: &Url,
    media_file_path: &std::path::Path,
    file_size: u64,
    tx: &UnboundedSender<Result<DownloadChunk, Status>>,
) -> Result<(), Status> {
    debug!(url = %url, path = %media_file_path.display(), file_size, "Starting media stream to client");
    let stream_started_at = Instant::now();
    let total_streamed = stream_media_file(media_file_path, tx).await?;

    info!(
        url = %url,
        bytes_streamed = total_streamed,
        elapsed_ms = stream_started_at.elapsed().as_millis(),
        "Finished streaming downloaded media"
    );
    Ok(())
}

fn map_playlist_response(playlist: Playlist, max_file_size: u64) -> Result<MediaInfoResponse, Status> {
    let mut entries = Vec::with_capacity(playlist.inner.len());

    for (media, mut formats) in playlist.inner {
        formats.retain(|(format, _)| match format.filesize_approx {
            Some(size) => size <= max_file_size,
            None => true,
        });

        if formats.is_empty() {
            warn!(
                media_id = %media.id,
                url = %media.webpage_url,
                max_file_size,
                "No media formats remained after filtering download candidates"
            );
            return Err(Status::invalid_argument("No downloadable formats fit max file size"));
        }

        entries.push(MediaEntry {
            id: media.id,
            display_id: media.display_id,
            webpage_url: media.webpage_url.to_string(),
            direct_url: media.direct_url.map(|url| url.to_string()),
            title: media.title,
            uploader: media.uploader,
            duration: media.duration,
            playlist_index: i32::from(media.playlist_index),
            domain: media.webpage_url.domain().map(ToOwned::to_owned),
            audio_language: media.language,
            formats: formats
                .into_iter()
                .map(|(format, raw_info_json)| MediaFormatEntry {
                    format_id: format.format_id,
                    ext: format.ext,
                    width: format.width,
                    height: format.height,
                    aspect_ratio: format.aspect_ratio,
                    filesize_approx: format.filesize_approx,
                    raw_info_json,
                })
                .collect(),
        });
    }

    Ok(MediaInfoResponse { entries })
}

#[allow(clippy::result_large_err)]
fn reject_active_livestreams(playlist: &Playlist) -> Result<(), Status> {
    if playlist.inner.iter().any(|(media, _)| media.is_active_livestream()) {
        return Err(Status::invalid_argument("Livestream downloads are not supported"));
    }
    Ok(())
}

// Builds a single-photo playlist for a snapsave image: the direct CDN URL is fetched as-is at
// download time (`direct` marker), with the original post URL shown.
// Build a `direct`-marked video entry from a snapsave CDN URL, so the download path remuxes it with
// ffmpeg instead of invoking yt-dlp. Dimensions/duration are left to Telegram's own probing (the
// yt-dlp generic extractor didn't reliably provide them for these CDN URLs either).
fn synthesize_video(item: &ResolvedMedia, webpage_url: &Url) -> Result<Playlist, Status> {
    let name = snapsave_name(webpage_url);
    let media = MediaWithFormat {
        id: name.clone(),
        display_id: None,
        webpage_url: webpage_url.clone(),
        direct_url: Some(item.url.clone()),
        title: Some(name),
        language: None,
        uploader: None,
        duration: None,
        thumbnail: item.thumbnail.clone(),
        thumbnails: vec![],
        live_status: None,
        is_live: false,
        format_id: "direct".to_owned(),
        format_note: Some("direct".to_owned()),
        ext: "mp4".to_owned(),
        width: None,
        height: None,
        aspect_ratio: None,
        filesize_approx: None,
        playlist_index: Some(1),
        protocol: None,
        vcodec: None,
        acodec: None,
        direct_fetch: true,
    };
    let raw = serde_json::to_string(&media).map_err(|err| Status::internal(format!("Video info error: {err}")))?;
    Ok(Playlist::new(vec![(media, raw)]))
}

fn synthesize_photo(item: &ResolvedMedia, webpage_url: &Url) -> Result<Playlist, Status> {
    let raw = RawPhotoInfo {
        id: snapsave_name(webpage_url),
        display_id: None,
        webpage_url: webpage_url.clone(),
        direct_url: item.url.clone(),
        title: None,
        uploader: None,
        ext: photo_ext(&item.url),
        width: None,
        height: None,
        filesize_approx: None,
        playlist_index: 1,
        direct: true,
    };
    let entry = raw
        .into_playlist_entry()
        .map_err(|err| Status::internal(format!("Photo info error: {err}")))?;
    Ok(Playlist::new(vec![entry]))
}

// Image extension from a URL's last path segment, defaulting to `jpg`.
fn photo_ext(url: &Url) -> String {
    url.path_segments()
        .and_then(|segments| segments.filter(|segment| !segment.is_empty()).next_back())
        .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()))
        .filter(|ext| matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "heic"))
        .unwrap_or_else(|| "jpg".to_owned())
}

// A media name from a post URL: its last non-empty path segment (the reel/post shortcode).
fn snapsave_name(url: &Url) -> String {
    url.path_segments()
        .and_then(|segments| segments.filter(|segment| !segment.is_empty()).next_back())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "media".to_owned())
}

// Sets the top-level `ext` (so yt-dlp names the output with a real container) and, when given, the
// `thumbnail` in a `--load-info-json` blob. Returns the input unchanged if it isn't a JSON object.
fn patch_info_json(raw: &str, ext: &str, thumbnail: Option<&Url>) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return raw.to_owned();
    };
    let Some(object) = value.as_object_mut() else {
        return raw.to_owned();
    };
    object.insert("ext".to_owned(), serde_json::Value::String(ext.to_owned()));
    if let Some(thumbnail) = thumbnail {
        object.insert("thumbnail".to_owned(), serde_json::Value::String(thumbnail.to_string()));
    }
    serde_json::to_string(&value).unwrap_or_else(|_| raw.to_owned())
}

// Selects snapsave carousel URLs by the `items` range (1-based start:count:step), mirroring playlist
// range selection on the node.
fn select_by_range<T>(items: Vec<T>, range: &Range) -> Vec<T> {
    items
        .into_iter()
        .enumerate()
        .filter(|(index, _)| {
            i16::try_from(*index + 1)
                .is_ok_and(|position| position >= range.start && position <= range.count && (position - range.start) % range.step == 0)
        })
        .map(|(_, item)| item)
        .collect()
}

#[allow(clippy::result_large_err)]
fn parse_range(range: Option<proto::downloader::Range>) -> Result<Range, Status> {
    let Some(range) = range else {
        return Ok(Range::default());
    };
    let start = positive_i16(range.start, "playlist_range.start")?;
    let count = positive_i16(range.count, "playlist_range.count")?;
    let step = positive_i16(range.step, "playlist_range.step")?;
    if start > count {
        return Err(Status::invalid_argument("playlist_range.start must be <= playlist_range.count"));
    }

    let mut range = Range { start, count, step };
    range.normalize();
    Ok(range)
}

#[allow(clippy::result_large_err)]
fn positive_i16(value: i32, field: &str) -> Result<i16, Status> {
    let value = i16::try_from(value).map_err(|_| Status::invalid_argument(format!("Invalid {field}")))?;
    if value <= 0 {
        return Err(Status::invalid_argument(format!("{field} must be positive")));
    }
    Ok(value)
}

fn parse_section(section: Option<Section>) -> Option<Sections> {
    section.map(|section| Sections {
        start: section.start,
        end: section.end,
    })
}

fn download_error_status(err: ytdl::DownloadErrorKind) -> Status {
    match err {
        ytdl::DownloadErrorKind::Retryable(kind) => Status::aborted(kind.to_string()),
        err => Status::internal(err.to_string()),
    }
}

fn parse_language(audio_language: &str) -> Language {
    if audio_language.trim().is_empty() {
        Language::default()
    } else {
        Language {
            language: Some(audio_language.to_owned()),
        }
    }
}

#[allow(clippy::result_large_err)]
fn parse_format_strategy(media_type: &str, audio_ext: &str) -> Result<FormatStrategy, Status> {
    match media_type {
        "video" => Ok(FormatStrategy::VideoAndAudio),
        "audio" => Ok(FormatStrategy::AudioOnly {
            audio_ext: if audio_ext.is_empty() { AUDIO_EXT } else { audio_ext }.to_owned(),
        }),
        _ => Err(Status::invalid_argument("Invalid media_type")),
    }
}

#[allow(clippy::result_large_err)]
fn parse_url(raw: &str) -> Result<Url, Status> {
    Url::parse(raw).map_err(|err| Status::invalid_argument(format!("Invalid url: {err}")))
}

fn resolve_max_file_size(request_max_file_size: u64, node_max_file_size: u64) -> u64 {
    match request_max_file_size {
        0 => node_max_file_size,
        value => value.min(node_max_file_size),
    }
}

async fn download_thumbnail(thumb_urls: &[Url], thumb_file_path: &std::path::Path) -> bool {
    for thumb_url in thumb_urls {
        match download_and_convert(thumb_url.as_str(), thumb_file_path, FFMPEG_PATH, THUMBNAIL_TIMEOUT_SECS).await {
            Ok(()) => {
                info!(thumbnail_url = %thumb_url, "Thumbnail downloaded");
                return true;
            }
            Err(err) => {
                warn!(%err, thumbnail_url = %thumb_url, "Thumbnail download failed");
            }
        }
    }
    false
}

async fn stream_thumbnail_file(
    thumb_file_path: &std::path::Path,
    tx: &UnboundedSender<Result<DownloadChunk, Status>>,
) -> Result<(), Status> {
    match File::open(thumb_file_path).await {
        Ok(mut file) => {
            let mut buffer = vec![0u8; STREAM_CHUNK_SIZE];
            loop {
                let read = match file.read(&mut buffer).await {
                    Ok(read) => read,
                    Err(err) => {
                        warn!(%err, path = %thumb_file_path.display(), "Thumbnail read failed");
                        break;
                    }
                };
                if read == 0 {
                    break;
                }
                send_chunk(
                    tx,
                    DownloadChunk {
                        payload: Some(Payload::ThumbnailData(buffer[..read].to_vec())),
                    },
                )
                .await?;
            }
        }
        Err(err) => {
            warn!(%err, path = %thumb_file_path.display(), "Thumbnail open failed");
        }
    }
    Ok(())
}

async fn stream_media_file(media_file_path: &std::path::Path, tx: &UnboundedSender<Result<DownloadChunk, Status>>) -> Result<u64, Status> {
    let mut file = File::open(media_file_path)
        .await
        .map_err(|err| Status::internal(format!("Open file error: {err}")))?;
    let mut buffer = vec![0u8; STREAM_CHUNK_SIZE];
    let mut total_streamed = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|err| Status::internal(format!("Read file error: {err}")))?;
        if read == 0 {
            break;
        }
        total_streamed += read as u64;
        send_chunk(
            tx,
            DownloadChunk {
                payload: Some(Payload::Data(buffer[..read].to_vec())),
            },
        )
        .await?;
    }
    Ok(total_streamed)
}

async fn file_size(path: &std::path::Path) -> Option<u64> {
    tokio::fs::metadata(path).await.ok().map(|metadata| metadata.len())
}

async fn resolve_media_file_path(
    output_dir_path: &std::path::Path,
    expected_media_file_path: &std::path::Path,
) -> Result<std::path::PathBuf, Status> {
    if tokio::fs::try_exists(expected_media_file_path)
        .await
        .map_err(|err| Status::internal(format!("Exists check error: {err}")))?
    {
        return Ok(expected_media_file_path.to_path_buf());
    }

    let mut dir = tokio::fs::read_dir(output_dir_path)
        .await
        .map_err(|err| Status::internal(format!("Read dir error: {err}")))?;
    let mut fallback_path = None;
    let mut fallback_size = 0_u64;

    while let Some(entry) = dir
        .next_entry()
        .await
        .map_err(|err| Status::internal(format!("Read dir entry error: {err}")))?
    {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if !file_name.starts_with("media.") || matches!(file_name, "media.info.json" | "media.jpg") {
            continue;
        }

        let metadata = entry
            .metadata()
            .await
            .map_err(|err| Status::internal(format!("Entry metadata error: {err}")))?;
        if !metadata.is_file() {
            continue;
        }

        let size = metadata.len();
        if fallback_path.is_none() || size > fallback_size {
            fallback_size = size;
            fallback_path = Some(path);
        }
    }

    let Some(actual_media_file_path) = fallback_path else {
        return Err(Status::internal("Metadata error: downloaded media file not found"));
    };

    warn!(
        expected_path = %expected_media_file_path.display(),
        actual_path = %actual_media_file_path.display(),
        file_size = fallback_size,
        "Using fallback downloaded media path"
    );

    Ok(actual_media_file_path)
}

#[allow(clippy::result_large_err)]
async fn send_chunk(tx: &UnboundedSender<Result<DownloadChunk, Status>>, chunk: DownloadChunk) -> Result<(), Status> {
    tx.send(Ok(chunk)).map_err(|_| Status::cancelled("Client disconnected"))
}

#[allow(clippy::result_large_err)]
async fn send_status(tx: &UnboundedSender<Result<DownloadChunk, Status>>, status: Status) -> Result<(), Status> {
    tx.send(Err(status)).map_err(|_| Status::cancelled("Client disconnected"))
}

#[allow(clippy::cast_possible_truncation)]
fn resolve_download_duration(media_duration: Option<f32>, section: Option<&Sections>) -> Option<i64> {
    let media_duration_secs = || media_duration.map(|duration| duration as i64);
    match section {
        None | Some(Sections { start: None, end: None }) => media_duration_secs(),
        Some(Sections {
            start: Some(start),
            end: Some(end),
        }) => Some(i64::from((end - start).max(0))),
        Some(Sections {
            start: Some(start),
            end: None,
        }) => media_duration_secs().map(|duration| (duration - i64::from(*start)).max(0)),
        Some(Sections {
            start: None,
            end: Some(end),
        }) => Some(i64::from((*end).max(0))),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_range, reject_active_livestreams, resolve_download_duration};
    use crate::entities::{Media, Playlist, Sections};
    use tonic::Code;
    use url::Url;

    #[test]
    fn parse_range_rejects_non_positive_values() {
        for (start, count, step) in [(0, 5, 1), (-1, 5, 1), (1, 0, 1), (1, -5, 1), (1, 5, 0), (1, 5, -1)] {
            let err = parse_range(Some(proto::downloader::Range { start, count, step })).unwrap_err();
            assert_eq!(
                err.code(),
                Code::InvalidArgument,
                "expected InvalidArgument for ({start}, {count}, {step})"
            );
        }
    }

    #[test]
    fn parse_range_rejects_inverted_start_count() {
        let err = parse_range(Some(proto::downloader::Range {
            start: 5,
            count: 1,
            step: 1,
        }))
        .unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn parse_range_accepts_valid_input() {
        let range = parse_range(Some(proto::downloader::Range {
            start: 1,
            count: 5,
            step: 2,
        }))
        .unwrap();
        assert_eq!(range.start, 1);
        assert_eq!(range.count, 5);
        assert_eq!(range.step, 2);
    }

    #[test]
    fn parse_range_defaults_when_absent() {
        let range = parse_range(None).unwrap();
        assert_eq!(range, crate::entities::Range::default());
    }

    #[test]
    fn uses_original_duration_without_crop() {
        assert_eq!(resolve_download_duration(Some(125.9), None), Some(125));
    }

    #[test]
    fn uses_explicit_crop_range_duration() {
        assert_eq!(
            resolve_download_duration(
                Some(300.0),
                Some(&Sections {
                    start: Some(60),
                    end: Some(150),
                }),
            ),
            Some(90)
        );
    }

    #[test]
    fn uses_media_duration_for_open_ended_crop() {
        assert_eq!(
            resolve_download_duration(
                Some(300.0),
                Some(&Sections {
                    start: Some(180),
                    end: None,
                }),
            ),
            Some(120)
        );
    }

    #[test]
    fn uses_end_as_duration_for_leading_crop() {
        assert_eq!(
            resolve_download_duration(
                Some(300.0),
                Some(&Sections {
                    start: None,
                    end: Some(75),
                }),
            ),
            Some(75)
        );
    }

    #[test]
    fn rejects_active_livestream_playlist() {
        let playlist = Playlist {
            inner: vec![(
                Media {
                    id: "id".into(),
                    display_id: None,
                    webpage_url: Url::parse("https://www.youtube.com/watch?v=test").unwrap(),
                    direct_url: None,
                    title: Some("title".into()),
                    language: None,
                    uploader: None,
                    duration: None,
                    playlist_index: 1,
                    thumbnail: None,
                    thumbnails: vec![],
                    live_status: Some("is_live".into()),
                    is_live: true,
                },
                vec![],
            )],
        };

        let err = reject_active_livestreams(&playlist).unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "Livestream downloads are not supported");
    }

    #[test]
    fn allows_non_live_playlist() {
        let playlist = Playlist {
            inner: vec![(
                Media {
                    id: "id".into(),
                    display_id: None,
                    webpage_url: Url::parse("https://www.youtube.com/watch?v=test").unwrap(),
                    direct_url: None,
                    title: Some("title".into()),
                    language: None,
                    uploader: None,
                    duration: Some(120.0),
                    playlist_index: 1,
                    thumbnail: None,
                    thumbnails: vec![],
                    live_status: Some("was_live".into()),
                    is_live: false,
                },
                vec![],
            )],
        };

        assert!(reject_active_livestreams(&playlist).is_ok());
    }
}
