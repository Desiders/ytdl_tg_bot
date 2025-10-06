use crate::{
    config::{YtDlpConfig, YtPotProviderConfig},
    entities::{AudioAndFormat, AudioInFS, Cookies},
    interactors::Interactor,
    services::{download_audio_to_path, download_thumbnail_to_path, get_best_thumbnail_path_in_dir},
};

use std::io;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing::instrument;
use url::Url;

const DOWNLOAD_TIMEOUT: u64 = 180;

#[derive(thiserror::Error, Debug)]
pub enum DownloadAudioErrorKind {
    #[error("Ytdlp error: {0}")]
    Ytdlp(io::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadAudioPlaylistErrorKind {
    #[error("Channel error: {0}")]
    Channel(#[from] mpsc::error::SendError<(usize, Result<AudioInFS, DownloadAudioErrorKind>)>),
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
            let thumbnail_path = download_thumbnail_to_path(thumbnail_url, &video.id, self.temp_dir.path()).await;
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
}

impl DownloadAudioPlaylist {
    pub const fn new(yt_dlp_cfg: YtDlpConfig, yt_pot_provider_cfg: YtPotProviderConfig, cookies: Cookies) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
        }
    }
}

pub struct DownloadAudioPlaylistInput<'a> {
    pub url: Url,
    pub audios_and_formats: Box<[(AudioAndFormat<'a>, TempDir)]>,
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

        for (index, (AudioAndFormat { video, format }, temp_dir)) in audios_and_formats.iter().enumerate() {
            let extension = format.codec.get_extension();
            let file_path = temp_dir.as_ref().join(format!("{video_id}.{extension}", video_id = video.id));

            let (thumbnail_path, download_thumbnails) = if let Some(thumbnail_url) = video.thumbnail_url(host.as_ref()) {
                let thumbnail_path = download_thumbnail_to_path(thumbnail_url, &video.id, temp_dir.path()).await;
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
                        get_best_thumbnail_path_in_dir(temp_dir.path()).ok().flatten()
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
