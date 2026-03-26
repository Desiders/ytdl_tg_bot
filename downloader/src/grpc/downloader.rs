use std::{
    fmt, fs,
    pin::Pin,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Instant,
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
use tracing::{error, info, warn};
use url::Url;
use ytdl_tg_bot_proto::downloader::{
    download_chunk::Payload, downloader_server::Downloader, DownloadChunk, DownloadMeta, DownloadRequest, MediaEntry, MediaFormatEntry,
    MediaInfoRequest, MediaInfoResponse, Section,
};

use crate::{
    config::{YtDlpConfig, YtPotProviderConfig},
    entities::{Cookies, Language, Media, MediaFormat, MediaWithFormat, Playlist, Range, Sections},
    services::{
        download_and_convert, embed_thumbnail,
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
        let format_strategy = parse_format_strategy(&request.media_type, &request.audio_language)?;
        let allow_playlist = !playlist_range.is_single_element();
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        info!(
            url = %url,
            media_type = %request.media_type,
            audio_language = %request.audio_language,
            allow_playlist,
            has_cookie = cookie.is_some(),
            "Fetching media info"
        );

        let playlist = ytdl::get_media_info(
            url.as_str(),
            &format_strategy,
            &audio_language,
            &self.yt_dlp_cfg.executable_path,
            &self.yt_pot_provider_cfg.url,
            &playlist_range,
            allow_playlist,
            GET_INFO_TIMEOUT_SECS,
            cookie,
        )
        .await
        .map_err(|err| {
            error!(url = %url, media_type = %request.media_type, %err, "Get media info failed");
            Status::internal(err.to_string())
        })?;

        let entries_count = playlist.inner.len();
        info!(
            url = %url,
            media_type = %request.media_type,
            entries_count,
            elapsed_ms = started_at.elapsed().as_millis(),
            "Fetched media info"
        );

        Ok(Response::new(map_playlist_response(playlist)))
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
        let yt_pot_provider_cfg = self.yt_pot_provider_cfg.clone();
        let cookies = self.cookies.clone();

        let (tx, rx) = mpsc::unbounded_channel();
        let error_tx = tx.clone();
        tokio::spawn(async move {
            let started_at = Instant::now();
            if let Err(status) = stream_download(request, yt_dlp_cfg, yt_pot_provider_cfg, cookies, tx).await {
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
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
    tx: UnboundedSender<Result<DownloadChunk, Status>>,
) -> Result<(), Status> {
    let url = parse_url(&request.url)?;
    let section = parse_section(request.section);
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
    let effective_max_file_size = resolve_max_file_size(request.max_file_size, yt_dlp_cfg.max_file_size);
    let host = url.host();
    let cookie = cookies.get_path_by_optional_host(host.as_ref()).cloned();

    info!(
        url = %url,
        format_id = %request.format_id,
        media_type = %requested_media_kind,
        has_cookie = cookie.is_some(),
        has_section = section.is_some(),
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

    let thumbnail_downloaded = download_thumbnail(&thumb_urls, &thumb_file_path).await;

    if let Err(status) = send_chunk(
        &tx,
        DownloadChunk {
            payload: Some(Payload::Meta(DownloadMeta {
                ext: ext.clone(),
                width: format.width,
                height: format.height,
                duration: media.duration.map(duration_seconds_to_i64),
                has_thumbnail: thumbnail_downloaded,
            })),
        },
    ) {
        return Err(status);
    }

    if thumbnail_downloaded {
        stream_thumbnail_file(&thumb_file_path, &tx).await?;
    }

    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<String>();
    let progress_forwarder = {
        let tx = tx.clone();
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
    };

    ytdl::download_media(
        strategy,
        &request.format_id,
        section.as_ref(),
        effective_max_file_size,
        output_dir_path,
        &info_file_path,
        &yt_dlp_cfg.executable_path,
        &yt_pot_provider_cfg.url,
        DOWNLOAD_TIMEOUT_SECS,
        cookie.as_ref(),
        Some(&progress_tx),
    )
    .await
    .map_err(|err| Status::internal(err.to_string()))?;
    drop(progress_tx);
    let _ = progress_forwarder.await;

    info!(url = %url, path = %media_file_path.display(), "Media downloaded, preparing stream");

    if thumbnail_downloaded {
        match embed_thumbnail(&media_file_path, &thumb_file_path).await {
            Ok(()) => {
                info!("Thumbnail embedded");
            }
            Err(err) => {
                warn!(%err, "Thumbnail embed failed");
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

    let total_streamed = stream_media_file(&media_file_path, &tx).await?;

    info!(url = %url, bytes_streamed = total_streamed, "Finished streaming downloaded media");
    drop(temp_dir);
    Ok(())
}

fn map_playlist_response(playlist: Playlist) -> MediaInfoResponse {
    let entries = playlist
        .inner
        .into_iter()
        .map(|(media, formats)| MediaEntry {
            id: media.id,
            display_id: media.display_id,
            webpage_url: media.webpage_url.to_string(),
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
        })
        .collect();

    MediaInfoResponse { entries }
}

#[allow(clippy::result_large_err)]
fn parse_range(range: Option<ytdl_tg_bot_proto::downloader::Range>) -> Result<Range, Status> {
    let Some(range) = range else {
        return Ok(Range::default());
    };
    let step = i16::try_from(range.step).map_err(|_| Status::invalid_argument("Invalid playlist_range.step"))?;
    if step == 0 {
        return Err(Status::invalid_argument("Invalid playlist_range.step"));
    }

    let mut range = Range {
        start: i16::try_from(range.start).map_err(|_| Status::invalid_argument("Invalid playlist_range.start"))?,
        count: i16::try_from(range.count).map_err(|_| Status::invalid_argument("Invalid playlist_range.count"))?,
        step,
    };
    range.normalize();
    Ok(range)
}

fn parse_section(section: Option<Section>) -> Option<Sections> {
    section.map(|section| Sections {
        start: section.start,
        end: section.end,
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
