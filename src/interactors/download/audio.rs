use crate::{
    config::{YtDlpConfig, YtPotProviderConfig},
    entities::{AudioAndFormat, AudioInFS, Cookies},
    interactors::Interactor,
    services::{convert_to_jpg, download_audio_to_path, get_best_thumbnail_path_in_dir},
    utils::format_error_report,
};

use nix::errno::Errno;
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::TempDir;
use tokio::{sync::mpsc, time::timeout};
use tracing::{event, instrument, Level};
use url::Url;

const DOWNLOAD_TIMEOUT: u64 = 180;
const RANGE_CHUNK_SIZE: i32 = 1024 * 1024 * 10;

#[derive(thiserror::Error, Debug)]
pub enum DownloadAudioErrorKind {
    #[error("Ytdlp error: {0}")]
    Ytdlp(io::Error),
    #[error("Ffmpeg error: {0}")]
    Ffmpeg(io::Error),
    #[error("Pipe error: {0}")]
    Pipe(Errno),
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadAudioPlaylistErrorKind {
    #[error("Channel error: {0}")]
    Channel(#[from] mpsc::error::SendError<(usize, Result<AudioInFS, DownloadAudioErrorKind>)>),
    #[error("Ytdlp error: {0}")]
    Ytdlp(io::Error),
    #[error("Ffmpeg error: {0}")]
    Ffmpeg(io::Error),
    #[error("Pipe error: {0}")]
    Pipe(Errno),
}

pub struct DownloadAudio {
    yt_dlp_cfg: YtDlpConfig,
    yt_pot_provider_cfg: YtPotProviderConfig,
    cookies: Cookies,
    temp_dir: TempDir,
}

impl DownloadAudio {
    pub const fn new(yt_dlp_cfg: YtDlpConfig, yt_pot_provider_cfg: YtPotProviderConfig, cookies: Cookies, temp_dir: TempDir) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
            temp_dir,
        }
    }
}

pub struct DownloadAudioInput<'a> {
    pub url: Url,
    pub video_and_format: AudioAndFormat<'a>,
}

impl Interactor for DownloadAudio {
    type Input<'a> = DownloadAudioInput<'a>;
    type Output = AudioInFS;
    type Err = DownloadAudioErrorKind;

    #[instrument(skip(self))]
    async fn execute(
        &mut self,
        DownloadAudioInput {
            video_and_format: AudioAndFormat { video, format },
            url,
        }: Self::Input<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let extension = format.codec.get_extension();
        let file_path = self.temp_dir.as_ref().join(format!("{video_id}.{extension}", video_id = video.id));
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        let (thumbnail_path, download_thumbnails) = if let Some(thumbnail_url) = video.thumbnail_url(host.as_ref()) {
            let thumbnail_path = get_thumbnail_path(thumbnail_url, &video.id, self.temp_dir.path()).await;
            (thumbnail_path, false)
        } else {
            (None, true)
        };

        download_audio_to_path(
            self.yt_dlp_cfg.executable_path.as_ref(),
            &video.original_url,
            self.yt_pot_provider_cfg.url.as_ref(),
            format.url,
            extension,
            &file_path,
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
                    get_best_thumbnail_path_in_dir(self.temp_dir.path()).ok().flatten()
                } else {
                    None
                }
            }
        };

        Ok(AudioInFS::new(file_path, thumbnail_path))
    }
}

pub struct DownloadAudioPlaylist {
    yt_dlp_cfg: YtDlpConfig,
    yt_pot_provider_cfg: YtPotProviderConfig,
    cookies: Cookies,
    temp_dir: TempDir,
}

impl DownloadAudioPlaylist {
    pub const fn new(yt_dlp_cfg: YtDlpConfig, yt_pot_provider_cfg: YtPotProviderConfig, cookies: Cookies, temp_dir: TempDir) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
            temp_dir,
        }
    }
}

pub struct DownloadAudioPlaylistInput<'a> {
    pub url: Url,
    pub audios_and_formats: Box<[AudioAndFormat<'a>]>,
    pub sender: mpsc::Sender<(usize, Result<AudioInFS, DownloadAudioErrorKind>)>,
}

impl Interactor for DownloadAudioPlaylist {
    type Input<'a> = DownloadAudioPlaylistInput<'a>;
    type Output = ();
    type Err = DownloadAudioPlaylistErrorKind;

    #[instrument(skip(self, audios_and_formats, sender))]
    async fn execute(
        &mut self,
        DownloadAudioPlaylistInput {
            url,
            audios_and_formats,
            sender,
        }: Self::Input<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        for (index, AudioAndFormat { video, format }) in audios_and_formats.iter().enumerate() {
            let extension = format.codec.get_extension();
            let file_path = self.temp_dir.as_ref().join(format!("{video_id}.{extension}", video_id = video.id));

            let (thumbnail_path, download_thumbnails) = if let Some(thumbnail_url) = video.thumbnail_url(host.as_ref()) {
                let thumbnail_path = get_thumbnail_path(thumbnail_url, &video.id, self.temp_dir.path()).await;
                (thumbnail_path, false)
            } else {
                (None, true)
            };

            if let Err(err) = download_audio_to_path(
                self.yt_dlp_cfg.executable_path.as_ref(),
                &video.original_url,
                self.yt_pot_provider_cfg.url.as_ref(),
                format.url,
                extension,
                &file_path,
                DOWNLOAD_TIMEOUT,
                download_thumbnails,
                cookie,
            )
            .await
            {
                sender.send((index, Err(DownloadAudioErrorKind::Ytdlp(err)))).await?;
                continue;
            }

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

            sender.send((index, Ok(AudioInFS::new(file_path, thumbnail_path)))).await?;
        }

        Ok(())
    }
}

#[instrument(skip(temp_dir_path), fields(url = url.as_ref(), id = id.as_ref()))]
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
