use crate::{
    config::{YtDlpConfig, YtPotProviderConfig},
    entities::{Cookies, VideoAndFormat, VideoInFS},
    interactors::Interactor,
    services::{download_thumbnail_to_path, download_to_pipe, download_video_to_path, get_best_thumbnail_path_in_dir, merge_streams},
    utils::format_error_report,
};

use bytes::Bytes;
use futures_util::StreamExt as _;
use nix::{
    errno::Errno,
    fcntl::{fcntl, FcntlArg::F_SETFD, FdFlag},
    unistd::pipe,
};
use reqwest::Client;
use std::{fs::File, io, path::PathBuf, sync::Arc, time::Duration};
use tokio::{io::AsyncWriteExt as _, sync::mpsc, time::timeout};
use tracing::{event, instrument, Level};
use url::Url;

const DOWNLOAD_TIMEOUT: u64 = 180;
const RANGE_CHUNK_SIZE: i32 = 1024 * 1024 * 10;

#[derive(thiserror::Error, Debug)]
pub enum RangeDownloadErrorKind {
    #[error("Channel error: {0}")]
    Channel(#[from] mpsc::error::SendError<Bytes>),
    #[error("Request error: {0}")]
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
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadVideoPlaylistErrorKind {
    #[error("Channel error: {0}")]
    Channel(#[from] mpsc::error::SendError<(usize, Result<VideoInFS, DownloadVideoErrorKind>)>),
    #[error("Ytdlp error: {0}")]
    Ytdlp(io::Error),
    #[error("Ffmpeg error: {0}")]
    Ffmpeg(io::Error),
    #[error("Pipe error: {0}")]
    Pipe(Errno),
}

pub struct DownloadVideo {
    yt_dlp_cfg: Arc<YtDlpConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
    temp_dir_path: PathBuf,
}

impl DownloadVideo {
    pub const fn new(
        yt_dlp_cfg: Arc<YtDlpConfig>,
        yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
        cookies: Arc<Cookies>,
        temp_dir_path: PathBuf,
    ) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
            temp_dir_path,
        }
    }
}

pub struct DownloadVideoInput<'a> {
    pub url: &'a Url,
    pub video_and_format: VideoAndFormat<'a>,
}

impl<'a> DownloadVideoInput<'a> {
    pub const fn new(url: &'a Url, video_and_format: VideoAndFormat<'a>) -> Self {
        Self { url, video_and_format }
    }
}

impl Interactor for DownloadVideo {
    type Input<'a> = DownloadVideoInput<'a>;
    type Output = VideoInFS;
    type Err = DownloadVideoErrorKind;

    #[instrument(target = "download", skip_all)]
    async fn execute(
        &mut self,
        DownloadVideoInput {
            video_and_format: VideoAndFormat { video, format },
            url,
        }: Self::Input<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let extension = format.get_extension();
        let file_path = self.temp_dir_path.join(format!("{video_id}.{extension}", video_id = video.id));
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        if format.format_ids_are_equal() {
            event!(Level::DEBUG, "Formats are the same");

            let (thumbnail_path, download_thumbnails) = if let Some(thumbnail_url) = video.thumbnail_url(host.as_ref()) {
                let thumbnail_path = download_thumbnail_to_path(thumbnail_url, &video.id, &self.temp_dir_path).await;
                (thumbnail_path, false)
            } else {
                (None, true)
            };

            download_video_to_path(
                self.yt_dlp_cfg.executable_path.as_ref(),
                &video.original_url,
                self.yt_pot_provider_cfg.url.as_ref(),
                extension,
                &self.temp_dir_path,
                DOWNLOAD_TIMEOUT,
                download_thumbnails,
                cookie,
            )
            .await
            .map_err(Self::Err::Ytdlp)?;

            let thumbnail_path = match thumbnail_path {
                Some(url) => Some(url),
                None => {
                    if download_thumbnails {
                        get_best_thumbnail_path_in_dir(&self.temp_dir_path).ok().flatten()
                    } else {
                        None
                    }
                }
            };

            return Ok(VideoInFS::new(file_path, thumbnail_path));
        }

        event!(Level::DEBUG, "Formats are different");

        let (video_read_fd, video_write_fd) = pipe().map_err(Self::Err::Pipe)?;
        let (audio_read_fd, audio_write_fd) = pipe().map_err(Self::Err::Pipe)?;

        fcntl(&video_write_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(Self::Err::Pipe)?;
        fcntl(&audio_write_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(Self::Err::Pipe)?;

        let mut merge_child = merge_streams(&video_read_fd, &audio_read_fd, extension, &file_path).map_err(Self::Err::Ffmpeg)?;

        if let Some(filesize) = format.video_format.filesize_or_approx() {
            let (sender, mut receiver) = mpsc::unbounded_channel();

            let url = format.video_format.url.to_owned();
            tokio::spawn(async move {
                tokio::join!(
                    async move {
                        let _ = range_download_to_write(url, filesize, sender)
                            .await
                            .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)));
                    },
                    async move {
                        let mut writer = tokio::fs::File::from_std(File::from(video_write_fd));
                        while let Some(bytes) = receiver.recv().await {
                            let _ = writer
                                .write(&bytes)
                                .await
                                .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)));
                        }
                    }
                )
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
            let (sender, mut receiver) = mpsc::unbounded_channel();

            let url = format.audio_format.url.to_owned();
            tokio::spawn(async move {
                tokio::join!(
                    async move {
                        let _ = range_download_to_write(url, filesize, sender)
                            .await
                            .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)));
                    },
                    async move {
                        let mut writer = tokio::fs::File::from_std(File::from(audio_write_fd));
                        while let Some(bytes) = receiver.recv().await {
                            let _ = writer
                                .write(&bytes)
                                .await
                                .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)));
                        }
                    }
                )
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

        let thumbnail_path = if let Some(thumbnail_url) = video.thumbnail_url(host.as_ref()) {
            download_thumbnail_to_path(thumbnail_url, &video.id, &self.temp_dir_path).await
        } else {
            None
        };

        let exit_code = match timeout(Duration::from_secs(DOWNLOAD_TIMEOUT), merge_child.wait()).await {
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

        Ok(VideoInFS::new(file_path, thumbnail_path))
    }
}

pub struct DownloadVideoPlaylist {
    yt_dlp_cfg: Arc<YtDlpConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
}

impl DownloadVideoPlaylist {
    pub const fn new(yt_dlp_cfg: Arc<YtDlpConfig>, yt_pot_provider_cfg: Arc<YtPotProviderConfig>, cookies: Arc<Cookies>) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
        }
    }
}

pub struct DownloadVideoPlaylistInput<'a> {
    pub url: &'a Url,
    pub videos_and_formats: Box<[(VideoAndFormat<'a>, PathBuf)]>,
    pub sender: mpsc::UnboundedSender<(usize, Result<VideoInFS, DownloadVideoErrorKind>)>,
}

impl<'a> DownloadVideoPlaylistInput<'a> {
    pub fn new(
        url: &'a Url,
        videos_and_formats: Box<[(VideoAndFormat<'a>, PathBuf)]>,
    ) -> (Self, mpsc::UnboundedReceiver<(usize, Result<VideoInFS, DownloadVideoErrorKind>)>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                videos_and_formats,
                sender,
            },
            receiver,
        )
    }
}

impl Interactor for DownloadVideoPlaylist {
    type Input<'a> = DownloadVideoPlaylistInput<'a>;
    type Output = ();
    type Err = DownloadVideoPlaylistErrorKind;

    #[instrument(target = "download_playlist", skip_all)]
    async fn execute(
        &mut self,
        DownloadVideoPlaylistInput {
            url,
            videos_and_formats,
            sender,
        }: Self::Input<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        for (index, (VideoAndFormat { video, format }, temp_dir_path)) in videos_and_formats.iter().enumerate() {
            let extension = format.get_extension();
            let file_path = temp_dir_path.join(format!("{video_id}.{extension}", video_id = video.id));

            if format.format_ids_are_equal() {
                event!(Level::DEBUG, "Formats are the same");

                let (thumbnail_path, download_thumbnails) = if let Some(thumbnail_url) = video.thumbnail_url(host.as_ref()) {
                    let thumbnail_path = download_thumbnail_to_path(thumbnail_url, &video.id, temp_dir_path).await;
                    (thumbnail_path, false)
                } else {
                    (None, true)
                };

                if let Err(err) = download_video_to_path(
                    self.yt_dlp_cfg.executable_path.as_ref(),
                    &video.original_url,
                    self.yt_pot_provider_cfg.url.as_ref(),
                    extension,
                    temp_dir_path,
                    DOWNLOAD_TIMEOUT,
                    download_thumbnails,
                    cookie,
                )
                .await
                {
                    sender.send((index, Err(DownloadVideoErrorKind::Ytdlp(err))))?;
                    continue;
                };

                let thumbnail_path = match thumbnail_path {
                    Some(url) => Some(url),
                    None => {
                        if download_thumbnails {
                            get_best_thumbnail_path_in_dir(temp_dir_path).ok().flatten()
                        } else {
                            None
                        }
                    }
                };

                sender.send((index, Ok(VideoInFS::new(file_path, thumbnail_path))))?;
                continue;
            }

            event!(Level::DEBUG, "Formats are different");

            let (video_read_fd, video_write_fd) = pipe().map_err(Self::Err::Pipe)?;
            let (audio_read_fd, audio_write_fd) = pipe().map_err(Self::Err::Pipe)?;

            fcntl(&video_write_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(Self::Err::Pipe)?;
            fcntl(&audio_write_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(Self::Err::Pipe)?;

            let mut merge_child = merge_streams(&video_read_fd, &audio_read_fd, extension, &file_path).map_err(Self::Err::Ffmpeg)?;

            if let Some(filesize) = format.video_format.filesize_or_approx() {
                let (sender, mut receiver) = mpsc::unbounded_channel();

                let url = format.video_format.url.to_owned();
                tokio::spawn(async move {
                    tokio::join!(
                        async move {
                            let _ = range_download_to_write(url, filesize, sender)
                                .await
                                .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)));
                        },
                        async move {
                            let mut writer = tokio::fs::File::from_std(File::from(video_write_fd));
                            while let Some(bytes) = receiver.recv().await {
                                let _ = writer
                                    .write(&bytes)
                                    .await
                                    .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)));
                            }
                        }
                    )
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
                let (sender, mut receiver) = mpsc::unbounded_channel();

                let url = format.audio_format.url.to_owned();
                tokio::spawn(async move {
                    tokio::join!(
                        async move {
                            let _ = range_download_to_write(url, filesize, sender)
                                .await
                                .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)));
                        },
                        async move {
                            let mut writer = tokio::fs::File::from_std(File::from(audio_write_fd));
                            while let Some(bytes) = receiver.recv().await {
                                let _ = writer
                                    .write(&bytes)
                                    .await
                                    .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)));
                            }
                        }
                    )
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

            let thumbnail_path = if let Some(thumbnail_url) = video.thumbnail_url(host.as_ref()) {
                download_thumbnail_to_path(thumbnail_url, &video.id, temp_dir_path).await
            } else {
                None
            };

            let exit_code = match timeout(Duration::from_secs(DOWNLOAD_TIMEOUT), merge_child.wait()).await {
                Ok(Ok(exit_code)) => exit_code,
                Ok(Err(err)) => {
                    sender.send((index, Err(DownloadVideoErrorKind::Ffmpeg(err))))?;
                    continue;
                }
                Err(_) => {
                    sender.send((
                        index,
                        Err(DownloadVideoErrorKind::Ffmpeg(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "FFmpeg process timed out",
                        ))),
                    ))?;
                    continue;
                }
            };

            if !exit_code.success() {
                sender.send((
                    index,
                    Err(DownloadVideoErrorKind::Ffmpeg(io::Error::other(format!(
                        "FFmpeg exited with status `{exit_code}`"
                    )))),
                ))?;
                continue;
            }

            event!(Level::DEBUG, "Streams merged");

            sender.send((index, Ok(VideoInFS::new(file_path, thumbnail_path))))?;
        }

        Ok(())
    }
}

async fn range_download_to_write(
    url: impl AsRef<str>,
    filesize: f64,
    sender: mpsc::UnboundedSender<Bytes>,
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
                sender.send(chunk)?;
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
            sender.send(chunk)?;
        }

        start = end + 1;
        end += RANGE_CHUNK_SIZE;
    }

    Ok(())
}
