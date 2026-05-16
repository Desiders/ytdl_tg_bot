use std::{
    ffi::OsStr,
    fmt, fs,
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
        download_and_convert, embed_thumbnail,
        gallery_dl::{self, GetInfoErrorKind},
        ytdl::{self, FormatStrategy},
    },
};

const GET_INFO_TIMEOUT_SECS: u64 = 180;
const DOWNLOAD_TIMEOUT_SECS: u64 = 420;
const AUDIO_EXT: &str = "m4a";
const THUMBNAIL_TIMEOUT_SECS: u64 = 5;
const FFMPEG_PATH: &str = "/usr/bin/ffmpeg";
const STREAM_CHUNK_SIZE: usize = 256 * 1024;

type DownloadStream = Pin<Box<dyn Stream<Item = Result<DownloadChunk, Status>> + Send + 'static>>;

pub struct DownloaderService {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub gallery_dl_cfg: Arc<GalleryDlConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
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
        let effective_max_file_size = resolve_max_file_size(request.max_file_size, self.yt_dlp_cfg.max_file_size);

        info!(
            url = %url,
            media_type = %request.media_type,
            audio_language = %request.audio_language,
            allow_playlist,
            has_cookie = cookie.is_some(),
            max_file_size = effective_max_file_size,
            "Fetching media info"
        );

        let playlist = if request.media_type == "photo" {
            gallery_dl::get_media_info(
                url.as_str(),
                self.gallery_dl_cfg.as_ref(),
                &playlist_range,
                GET_INFO_TIMEOUT_SECS,
                cookie.as_deref(),
            )
            .await
            .map_err(|err| match err {
                GetInfoErrorKind::EmptyEntries => Status::not_found(err.to_string()),
                err => Status::internal(err.to_string()),
            })?
        } else {
            let format_strategy = parse_format_strategy(&request.media_type, &request.audio_language)?;
            let playlist = ytdl::get_media_info(
                url.as_str(),
                &format_strategy,
                &audio_language,
                self.yt_dlp_cfg.as_ref(),
                &self.yt_pot_provider_cfg.url,
                &playlist_range,
                allow_playlist,
                GET_INFO_TIMEOUT_SECS,
                cookie.as_deref(),
            )
            .await
            .map_err(|err| match err {
                ytdl::GetInfoErrorKind::Retryable(kind) => Status::aborted(kind.to_string()),
                err => Status::internal(err.to_string()),
            })?;

            reject_active_livestreams(&playlist)?;
            playlist
        };

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
        let cookies = self.cookies.clone();

        let (tx, rx) = mpsc::unbounded_channel();
        let error_tx = tx.clone();
        tokio::spawn(async move {
            let started_at = Instant::now();
            if let Err(status) = stream_download(request, yt_dlp_cfg, gallery_dl_cfg, yt_pot_provider_cfg, cookies, tx).await {
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
                    let _ = send_status(&error_tx, status);
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

#[allow(clippy::too_many_lines)]
async fn stream_download(
    request: DownloadRequest,
    yt_dlp_cfg: Arc<YtDlpConfig>,
    gallery_dl_cfg: Arc<GalleryDlConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
    tx: UnboundedSender<Result<DownloadChunk, Status>>,
) -> Result<(), Status> {
    let url = parse_url(&request.url)?;
    let section = parse_section(request.section);
    let effective_max_file_size = resolve_max_file_size(request.max_file_size, yt_dlp_cfg.max_file_size);
    let host = url.host();
    let cookie = cookies.get_path_by_optional_host(host.as_ref());

    if request.media_type == "photo" {
        return stream_photo_download(request, url, effective_max_file_size, gallery_dl_cfg, cookie, tx).await;
    }

    let media_with_format: MediaWithFormat =
        serde_json::from_str(&request.raw_info_json).map_err(|err| Status::invalid_argument(format!("Invalid info file error: {err}")))?;
    let media = Media::from(media_with_format.clone());
    let format = MediaFormat::from(media_with_format);
    let (requested_media_kind, ext, strategy) = {
        let requested_media = parse_request_media(&request.media_type, &request.audio_ext)?;
        (
            requested_media.to_string(),
            requested_media.output_ext(&format).to_owned(),
            parse_format_strategy(&request.media_type, &request.audio_ext)?,
        )
    };

    info!(
        url = %url,
        format_id = %request.format_id,
        media_type = %requested_media_kind,
        has_cookie = cookie.is_some(),
        has_section = section.is_some(),
        section_start = section.as_ref().and_then(|section| section.start),
        section_end = section.as_ref().and_then(|section| section.end),
        max_file_size = effective_max_file_size,
        "Starting media download"
    );

    let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(|err| Status::internal(format!("Temp dir error: {err}")))?;
    let output_dir_path = temp_dir.path();
    let info_file_path = output_dir_path.join("media.info.json");
    fs::write(&info_file_path, &request.raw_info_json).map_err(|err| Status::internal(format!("Info file error: {err}")))?;

    let media_file_path = output_dir_path.join(format!("media.{ext}"));
    let thumb_file_path = output_dir_path.join("media.jpg");
    let thumb_urls = media.get_thumb_urls(format.aspect_ration_kind());

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

    // Race the download against its first progress line. If yt-dlp fails before producing any progress
    // (auth/geo/format errors), `download_future` resolves first with an Err — we propagate it without
    // sending Meta so `with_node_failover` on the client can retry on another node. Once a progress line
    // arrives, the download has started successfully: send Meta so the bot can leave the "Preparing"
    // state, then stream the remaining progress in real time.
    let initial_progress = tokio::select! {
        progress = progress_rx.recv() => progress,
        res = &mut download_future => {
            res.map_err(download_error_status)?;
            None
        }
    };

    send_chunk(
        &tx,
        DownloadChunk {
            payload: Some(Payload::Meta(DownloadMeta {
                ext: ext.clone(),
                width: format.width,
                height: format.height,
                duration: resolve_download_duration(media.duration, section.as_ref()),
                has_thumbnail: thumbnail_downloaded,
            })),
        },
    )?;

    if thumbnail_downloaded {
        debug!(url = %url, path = %thumb_file_path.display(), "Starting thumbnail stream to client");
        stream_thumbnail_file(&thumb_file_path, &tx).await?;
        debug!(url = %url, path = %thumb_file_path.display(), "Finished thumbnail stream to client");
    }

    match initial_progress {
        Some(progress) => {
            send_chunk(
                &tx,
                DownloadChunk {
                    payload: Some(Payload::Progress(progress)),
                },
            )?;

            let progress_forwarder = spawn_progress_forwarder(progress_rx, tx.clone());
            // Race the download against client disconnection. If the client drops the stream,
            // dropping `download_future` here kills yt-dlp via `kill_on_drop` instead of letting
            // it run to completion and waste CPU/disk/bandwidth.
            tokio::select! {
                res = download_future => {
                    res.map_err(download_error_status)?;
                }
                () = tx.closed() => {
                    return Err(Status::cancelled("Client disconnected"));
                }
            }
            drop(progress_tx);
            let _ = progress_forwarder.await;
        }
        None => {
            // Download already completed inside the select!; drop the boxed future
            // to release the borrow on `progress_tx` / `temp_dir`.
            drop(download_future);
        }
    }

    let media_file_path = resolve_media_file_path(output_dir_path, &media_file_path).await?;

    if thumbnail_downloaded {
        let embed_started_at = Instant::now();
        let media_size_before_embed = file_size(&media_file_path).await;
        let thumbnail_size = file_size(&thumb_file_path).await;
        debug!(
            url = %url,
            media_path = %media_file_path.display(),
            thumbnail_path = %thumb_file_path.display(),
            ?media_size_before_embed,
            ?thumbnail_size,
            "Starting thumbnail embed"
        );
        match embed_thumbnail(&media_file_path, &thumb_file_path).await {
            Ok(()) => {
                let media_size_after_embed = file_size(&media_file_path).await;
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

    let metadata = tokio::fs::metadata(&media_file_path)
        .await
        .map_err(|err| Status::internal(format!("Metadata error: {err}")))?;
    if metadata.len() > effective_max_file_size {
        warn!(
            url = %url,
            file_size = metadata.len(),
            max_file_size = effective_max_file_size,
            "Downloaded file exceeds max file size"
        );
        return Err(Status::invalid_argument("File exceeds max file size"));
    }

    debug!(
        url = %url,
        path = %media_file_path.display(),
        file_size = metadata.len(),
        "Starting media stream to client"
    );
    let stream_started_at = Instant::now();
    let total_streamed = stream_media_file(&media_file_path, &tx).await?;

    info!(
        url = %url,
        bytes_streamed = total_streamed,
        elapsed_ms = stream_started_at.elapsed().as_millis(),
        "Finished streaming downloaded media"
    );
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

    send_chunk(
        &tx,
        DownloadChunk {
            payload: Some(Payload::Meta(DownloadMeta {
                ext: raw_info.ext.clone(),
                width: raw_info.width,
                height: raw_info.height,
                duration: None,
                has_thumbnail: false,
            })),
        },
    )?;

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

    let media_file_path = resolve_media_file_path(output_dir_path, &media_file_path).await?;
    let metadata = tokio::fs::metadata(&media_file_path)
        .await
        .map_err(|err| Status::internal(format!("Metadata error: {err}")))?;
    if metadata.len() > effective_max_file_size {
        warn!(
            url = %url,
            file_size = metadata.len(),
            max_file_size = effective_max_file_size,
            "Downloaded file exceeds max file size"
        );
        return Err(Status::invalid_argument("File exceeds max file size"));
    }

    debug!(
        url = %url,
        path = %media_file_path.display(),
        file_size = metadata.len(),
        "Starting media stream to client"
    );
    let stream_started_at = Instant::now();
    let total_streamed = stream_media_file(&media_file_path, &tx).await?;

    info!(
        url = %url,
        bytes_streamed = total_streamed,
        elapsed_ms = stream_started_at.elapsed().as_millis(),
        "Finished streaming downloaded media"
    );
    drop(temp_dir);
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

fn spawn_progress_forwarder(
    mut progress_rx: mpsc::UnboundedReceiver<String>,
    tx: UnboundedSender<Result<DownloadChunk, Status>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            if send_chunk(
                &tx,
                DownloadChunk {
                    payload: Some(Payload::Progress(progress)),
                },
            )
            .is_err()
            {
                break;
            }
        }
    })
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
fn parse_request_media<'a>(media_type: &'a str, audio_ext: &'a str) -> Result<RequestMedia<'a>, Status> {
    match media_type {
        "video" => Ok(RequestMedia::Video),
        "audio" => Ok(RequestMedia::Audio {
            audio_ext: if audio_ext.is_empty() { AUDIO_EXT } else { audio_ext },
        }),
        _ => Err(Status::invalid_argument("Invalid media type")),
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
                )?;
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
        )?;
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
fn send_chunk(tx: &UnboundedSender<Result<DownloadChunk, Status>>, chunk: DownloadChunk) -> Result<(), Status> {
    tx.send(Ok(chunk)).map_err(|_| Status::cancelled("Client disconnected"))
}

#[allow(clippy::result_large_err)]
fn send_status(tx: &UnboundedSender<Result<DownloadChunk, Status>>, status: Status) -> Result<(), Status> {
    tx.send(Err(status)).map_err(|_| Status::cancelled("Client disconnected"))
}

#[derive(Debug)]
enum RequestMedia<'a> {
    Video,
    Audio { audio_ext: &'a str },
}

impl RequestMedia<'_> {
    fn output_ext<'a>(&'a self, format: &'a MediaFormat) -> &'a str {
        match self {
            Self::Video => format.ext.as_str(),
            Self::Audio { audio_ext } => audio_ext,
        }
    }
}

impl fmt::Display for RequestMedia<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Video => "video",
                Self::Audio { .. } => "audio",
            }
        )
    }
}

#[allow(clippy::cast_possible_truncation)]
fn duration_seconds_to_i64(duration: f32) -> i64 {
    duration as i64
}

fn resolve_download_duration(media_duration: Option<f32>, section: Option<&Sections>) -> Option<i64> {
    match section {
        None | Some(Sections { start: None, end: None }) => media_duration.map(duration_seconds_to_i64),
        Some(Sections {
            start: Some(start),
            end: Some(end),
        }) => Some(i64::from((end - start).max(0))),
        Some(Sections {
            start: Some(start),
            end: None,
        }) => media_duration
            .map(duration_seconds_to_i64)
            .map(|duration| (duration - i64::from(*start)).max(0)),
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
