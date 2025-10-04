use futures_util::StreamExt as _;
use nix::{errno::Errno, unistd::pipe};
use reqwest::Client;
use std::{
    borrow::Cow,
    fs::File,
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::TempDir;
use tokio::{io::AsyncWriteExt, time::timeout};
use tracing::{event, instrument, Level};
use url::{Host, Url};

use crate::{
    config::{YtDlpConfig, YtPotProviderConfig, YtToolkitConfig},
    download::StreamErrorKind,
    entities::{combined_format::Format, Cookies, ShortInfo, Video, VideoInFS},
    handlers_utils::preferred_languages::PreferredLanguages,
    interactors::Interactor,
    services::{convert_to_jpg, download_to_pipe, download_video_to_path, get_best_thumbnail_path_in_dir, merge_streams, yt_toolkit},
    utils::format_error_report,
};

const DOWNLOAD_MEDIA_TIMEOUT: u64 = 180;
const RANGE_CHUNK_SIZE: i32 = 1024 * 1024 * 10;

pub struct DownloadVideo {
    yt_dlp_cfg: YtDlpConfig,
    yt_toolkit_cfg: YtToolkitConfig,
    yt_pot_provider_cfg: YtPotProviderConfig,
    cookies: Cookies,
    temp_dir: TempDir,
}

impl DownloadVideo {
    pub const fn new(
        yt_dlp_cfg: YtDlpConfig,
        yt_toolkit_cfg: YtToolkitConfig,
        yt_pot_provider_cfg: YtPotProviderConfig,
        cookies: Cookies,
        temp_dir: TempDir,
    ) -> Self {
        Self {
            yt_dlp_cfg,
            yt_toolkit_cfg,
            yt_pot_provider_cfg,
            cookies,
            temp_dir,
        }
    }
}

pub struct DownloadVideoInput<'a> {
    pub url: Url,
    pub video: &'a Video,
    pub format: Format<'a>,
    pub preferred_languages: PreferredLanguages,
}

pub struct DownloadVideoOutput {
    pub video_in_fs: VideoInFS,
}

#[derive(thiserror::Error, Debug)]
pub enum RangeDownloadErrorKind {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadVideoErrorKind {
    #[error("Ytdlp error: {0}")]
    Ytdlp(io::Error),
    #[error("Ffmpeg error: {0}")]
    Ffmpeg(io::Error),
    #[error("Pipe error: {0}")]
    Pipe(Errno),
    #[error("Range error: {0}")]
    Range(#[from] RangeDownloadErrorKind),
    #[error("Stream error: {0}")]
    Stream(#[from] StreamErrorKind),
}

impl Interactor for DownloadVideo {
    type Input<'a> = DownloadVideoInput<'a>;
    type Output = DownloadVideoOutput;
    type Err = DownloadVideoErrorKind;

    #[instrument(skip(self))]
    async fn execute(
        &mut self,
        DownloadVideoInput {
            video,
            format,
            url,
            preferred_languages,
        }: Self::Input<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let extension = format.get_extension();
        let file_path = self.temp_dir.as_ref().join(format!("{video_id}.{extension}", video_id = video.id));
        let name = video.title.as_deref().unwrap_or(video.id.as_ref());
        let host = url.host();
        let is_youtube = host.as_ref().is_some_and(|host| match host {
            Host::Domain(domain) => domain.contains("youtube") || *domain == "youtu.be",
            _ => false,
        });
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        if format.format_ids_are_equal() {
            event!(Level::DEBUG, "Video and audio formats are the same");

            let (thumbnail_path, download_thumbnails) =
                if let Some(thumbnail_url) = get_thumbnail_url(&video.clone().into(), self.yt_toolkit_cfg.url.as_ref(), is_youtube) {
                    let thumbnail_path = get_thumbnail_path(thumbnail_url, &video.id, self.temp_dir.path()).await;
                    (thumbnail_path, false)
                } else {
                    (None, true)
                };

            download_video_to_path(
                self.yt_dlp_cfg.executable_path.as_ref(),
                &video.original_url,
                self.yt_pot_provider_cfg.url.as_ref(),
                extension,
                self.temp_dir.path(),
                DOWNLOAD_MEDIA_TIMEOUT,
                download_thumbnails,
                cookie,
            )
            .await
            .map_err(Self::Err::Ytdlp)?;

            let thumbnail_path = match thumbnail_path {
                Some(url) => Some(url),
                None => {
                    if download_thumbnails {
                        get_best_thumbnail_path_in_dir(self.temp_dir.path()).ok().flatten()
                    } else {
                        None
                    }
                }
            };

            return Ok(DownloadVideoOutput {
                video_in_fs: VideoInFS::new(file_path, thumbnail_path),
            });
        }

        event!(Level::DEBUG, "Video and audio formats are different");

        let (video_read_fd, video_write_fd) = pipe().map_err(Self::Err::Pipe)?;
        let (audio_read_fd, audio_write_fd) = pipe().map_err(Self::Err::Pipe)?;

        let mut merge_child = merge_streams(&video_read_fd, &audio_read_fd, extension, &file_path).map_err(Self::Err::Ffmpeg)?;

        if let Some(filesize) = format.video_format.filesize_or_approx() {
            let url = format.video_format.url.to_owned();
            tokio::spawn(async move {
                let _ = range_download_to_write(url, filesize, tokio::fs::File::from_std(File::from(video_write_fd)))
                    .await
                    .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)));
            });
        } else {
            download_to_pipe(
                video_write_fd,
                self.yt_dlp_cfg.executable_path.as_ref(),
                &video.original_url,
                self.yt_pot_provider_cfg.url.as_ref(),
                format.video_format.id,
                cookie,
            )
            .map_err(Self::Err::Ytdlp)?;
        }

        if let Some(filesize) = format.audio_format.filesize_or_approx() {
            let url = format.audio_format.url.to_owned();
            tokio::spawn(async move {
                let _ = range_download_to_write(url, filesize, tokio::fs::File::from_std(File::from(audio_write_fd)))
                    .await
                    .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)));
            });
        } else {
            download_to_pipe(
                audio_write_fd,
                self.yt_dlp_cfg.executable_path.as_ref(),
                &video.original_url,
                self.yt_pot_provider_cfg.url.as_ref(),
                format.audio_format.id,
                cookie,
            )
            .map_err(Self::Err::Ytdlp)?;
        }

        let thumbnail_path =
            if let Some(thumbnail_url) = get_thumbnail_url(&video.clone().into(), self.yt_toolkit_cfg.url.as_ref(), is_youtube) {
                get_thumbnail_path(thumbnail_url, &video.id, self.temp_dir.path()).await
            } else {
                None
            };

        let exit_code = match timeout(Duration::from_secs(DOWNLOAD_MEDIA_TIMEOUT), merge_child.wait()).await {
            Ok(Ok(exit_code)) => exit_code,
            Ok(Err(err)) => {
                return Err(Self::Err::Ffmpeg(err));
            }
            Err(_) => {
                return Err(Self::Err::Ffmpeg(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "FFmpeg process timed out",
                )));
            }
        };

        if !exit_code.success() {
            return Err(Self::Err::Ffmpeg(io::Error::other(format!(
                "FFmpeg exited with status `{exit_code}`"
            ))));
        }

        event!(Level::DEBUG, "Streams merged");

        Ok(DownloadVideoOutput {
            video_in_fs: VideoInFS::new(file_path, thumbnail_path),
        })
    }
}

pub struct DownloadVideoPlaylist {}

impl DownloadVideoPlaylist {
    pub const fn new() -> Self {
        Self {}
    }
}

pub struct DownloadVideoPlaylistInput {}

pub struct DownloadVideoPlaylistOutput {}

#[derive(thiserror::Error, Debug)]
pub enum DownloadVideoPlaylistErrorKind {}

impl Interactor for DownloadVideoPlaylist {
    type Input<'a> = DownloadVideoPlaylistInput;
    type Output = DownloadVideoPlaylistOutput;
    type Err = DownloadVideoPlaylistErrorKind;

    async fn execute(&mut self, input: Self::Input<'_>) -> Result<Self::Output, Self::Err> {
        todo!()
    }
}

#[instrument(skip_all)]
fn get_thumbnail_url<'a>(video: &'a ShortInfo, yt_toolkit_api_url: impl AsRef<str>, is_youtube: bool) -> Option<Cow<'a, str>> {
    if is_youtube {
        match (video.width, video.height) {
            (Some(width), Some(height)) => match yt_toolkit::get_thumbnail_url(yt_toolkit_api_url.as_ref(), &video.id, width, height) {
                Ok(url) => Some(Cow::Owned(url)),
                Err(_) => video.thumbnail().map(Cow::Borrowed),
            },
            _ => video.thumbnail().map(Cow::Borrowed),
        }
    } else {
        video.thumbnail().map(Cow::Borrowed)
    }
}

#[instrument(skip_all)]
async fn get_thumbnail_path(url: impl AsRef<str>, id: impl AsRef<str>, temp_dir_path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = temp_dir_path.as_ref().join(format!("{}.jpg", id.as_ref()));

    match convert_to_jpg(url, &path).await {
        Ok(mut child) => match timeout(Duration::from_secs(10), child.wait()).await {
            Ok(Ok(status)) => {
                if status.success() {
                    Some(path)
                } else {
                    None
                }
            }
            Ok(Err(err)) => {
                event!(Level::ERROR, err = format_error_report(&err), "Failed to convert thumbnail");
                None
            }
            Err(_) => {
                event!(Level::WARN, "Convert thumbnail timed out");
                None
            }
        },
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Failed to convert thumbnail");
            None
        }
    }
}

async fn range_download_to_write<W: AsyncWriteExt + Unpin>(
    url: impl AsRef<str>,
    filesize: f64,
    mut write: W,
) -> Result<(), RangeDownloadErrorKind> {
    let client = Client::new();
    let url = url.as_ref();

    let mut start = 0;
    let mut end = RANGE_CHUNK_SIZE;

    loop {
        event!(Level::TRACE, start, end, "Download chunk");

        #[allow(clippy::cast_possible_truncation)]
        if end >= filesize as i32 {
            let mut stream = client
                .get(url)
                .header("Range", format!("bytes={start}-"))
                .send()
                .await?
                .bytes_stream();

            while let Some(chunk_res) = stream.next().await {
                let chunk = chunk_res?;
                write.write_all(&chunk).await?;
            }

            break;
        }

        let mut stream = client
            .get(url)
            .header("Range", format!("bytes={start}-{end}"))
            .send()
            .await?
            .bytes_stream();

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res?;
            write.write_all(&chunk).await?;
        }

        start = end + 1;
        end += RANGE_CHUNK_SIZE;
    }

    Ok(())
}
