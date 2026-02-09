use crate::{
    config::{TimeoutsConfig, YtDlpConfig, YtPotProviderConfig},
    entities::{Cookies, Media, MediaFormat, MediaInFS, RawMediaWithFormat},
    interactors::Interactor,
    services::{
        download_and_convert,
        ytdl::{self, download_media, FormatStrategy},
    },
};
use std::{fs, io, sync::Arc};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing::{debug, error, info, info_span, instrument, warn};
use url::Url;

#[derive(thiserror::Error, Debug)]
pub enum DownloadMediaErrorKind {
    #[error("Temp dir error: {0}")]
    TempDir(io::Error),
    #[error("Info file error: {0}")]
    InfoFile(io::Error),
    #[error("Channel error: {0}")]
    Channel(#[from] mpsc::error::SendError<ytdl::DownloadErrorKind>),
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadMediaPlaylistErrorKind {
    #[error("Temp dir error: {0}")]
    TempDir(io::Error),
    #[error("Info file error: {0}")]
    InfoFile(io::Error),
    #[error("Channel error: {0}")]
    ErrChannel(#[from] mpsc::error::SendError<Vec<ytdl::DownloadErrorKind>>),
    #[error("Channel error: {0}")]
    MediaChannel(#[from] mpsc::error::SendError<(MediaInFS, Media, MediaFormat)>),
}

pub struct DownloadMediaInput<'a> {
    url: &'a Url,
    media: &'a Media,
    formats: Vec<(MediaFormat, RawMediaWithFormat)>,
    err_sender: mpsc::UnboundedSender<ytdl::DownloadErrorKind>,
    progress_sender: Option<mpsc::UnboundedSender<String>>,
}

impl<'a> DownloadMediaInput<'a> {
    pub fn new_with_progress(
        url: &'a Url,
        media: &'a Media,
        formats: Vec<(MediaFormat, RawMediaWithFormat)>,
    ) -> (
        Self,
        mpsc::UnboundedReceiver<ytdl::DownloadErrorKind>,
        mpsc::UnboundedReceiver<String>,
    ) {
        let (err_sender, err_receiver) = mpsc::unbounded_channel();
        let (progress_sender, progress_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                media,
                formats,
                err_sender,
                progress_sender: Some(progress_sender),
            },
            err_receiver,
            progress_receiver,
        )
    }
}

pub struct DownloadMediaPlaylistInput<'a> {
    url: &'a Url,
    playlist: Vec<(Media, Vec<(MediaFormat, RawMediaWithFormat)>)>,
    media_sender: mpsc::UnboundedSender<(MediaInFS, Media, MediaFormat)>,
    errs_sender: Option<mpsc::UnboundedSender<Vec<ytdl::DownloadErrorKind>>>,
    progress_sender: Option<mpsc::UnboundedSender<String>>,
}

impl<'a> DownloadMediaPlaylistInput<'a> {
    #[allow(clippy::type_complexity)]
    pub fn new_with_progress(
        url: &'a Url,
        playlist: Vec<(Media, Vec<(MediaFormat, RawMediaWithFormat)>)>,
    ) -> (
        Self,
        mpsc::UnboundedReceiver<(MediaInFS, Media, MediaFormat)>,
        mpsc::UnboundedReceiver<Vec<ytdl::DownloadErrorKind>>,
        mpsc::UnboundedReceiver<String>,
    ) {
        let (media_sender, media_receiver) = mpsc::unbounded_channel();
        let (errs_sender, errs_receiver) = mpsc::unbounded_channel();
        let (progress_sender, progress_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                playlist,
                media_sender,
                errs_sender: Some(errs_sender),
                progress_sender: Some(progress_sender),
            },
            media_receiver,
            errs_receiver,
            progress_receiver,
        )
    }

    #[allow(clippy::type_complexity)]
    pub fn new(
        url: &'a Url,
        playlist: Vec<(Media, Vec<(MediaFormat, RawMediaWithFormat)>)>,
    ) -> (Self, mpsc::UnboundedReceiver<(MediaInFS, Media, MediaFormat)>) {
        let (media_sender, media_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                playlist,
                media_sender,
                errs_sender: None,
                progress_sender: None,
            },
            media_receiver,
        )
    }
}

pub struct DownloadVideo {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
    pub cookies: Arc<Cookies>,
}

impl Interactor<DownloadMediaInput<'_>> for &DownloadVideo {
    type Output = Option<(MediaInFS, MediaFormat)>;
    type Err = DownloadMediaErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaInput {
            url,
            media,
            formats,
            err_sender,
            progress_sender,
        }: DownloadMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let temp_dir = TempDir::new().map_err(Self::Err::TempDir)?;
        let output_dir_path = temp_dir.path();
        let thumb_file_path = output_dir_path.join(format!("{}.jpg", media.id));
        let info_file_path = output_dir_path.join(format!("{}.info.json", media.id));

        let mut thumn_is_downloaded = false;
        for (format, raw) in formats {
            let span = info_span!("iter", %format).entered();

            let format_id = &format.format_id;
            let max_file_size = self.yt_dlp_cfg.max_file_size;
            let media_file_path = output_dir_path.join(format!("{}.{}", media.id, format.ext));
            let info_file_path = &info_file_path;
            let executable_path = self.yt_dlp_cfg.executable_path.as_ref();
            let pot_provider_url = self.yt_pot_provider_cfg.url.as_ref();
            let timeout = self.timeouts_cfg.video_download;
            let host = url.host();
            let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

            debug!("Downloading video");
            let span = span.exit();

            fs::write(info_file_path, raw).map_err(Self::Err::InfoFile)?;

            if !thumn_is_downloaded {
                for thumb_url in media.get_thumb_urls(format.aspect_ration_kind()) {
                    match download_and_convert(thumb_url.as_str(), &thumb_file_path, "/usr/bin/ffmpeg", 5).await {
                        Ok(()) => {
                            let _guard = span.enter();
                            info!("Thumb downloaded");
                            thumn_is_downloaded = true;
                            break;
                        }
                        Err(err) => {
                            let _guard = span.enter();
                            warn!(%err, "Download thumb err");
                        }
                    }
                }
            }

            if let Err(err) = download_media(
                FormatStrategy::VideoAndAudio,
                format_id,
                max_file_size,
                output_dir_path,
                info_file_path,
                executable_path,
                pot_provider_url,
                timeout,
                cookie,
                progress_sender.as_ref(),
            )
            .await
            {
                let _guard = span.enter();
                error!(%err, "Download video err");
                err_sender.send(err)?;
                continue;
            }
            let _guard = span.enter();
            info!("Video downloaded");

            let media_in_fs = MediaInFS {
                path: media_file_path,
                thumb_path: if thumn_is_downloaded { Some(thumb_file_path) } else { None },
                temp_dir,
            };
            return Ok(Some((media_in_fs, format)));
        }
        Ok(None)
    }
}

pub struct DownloadAudio {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
    pub cookies: Arc<Cookies>,
}

impl Interactor<DownloadMediaInput<'_>> for &DownloadAudio {
    type Output = Option<(MediaInFS, MediaFormat)>;
    type Err = DownloadMediaErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaInput {
            url,
            media,
            formats,
            err_sender,
            progress_sender,
        }: DownloadMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let temp_dir = TempDir::new().map_err(Self::Err::TempDir)?;
        let audio_ext = "m4a";
        let output_dir_path = temp_dir.path();
        let thumb_file_path = output_dir_path.join(format!("{}.jpg", media.id));
        let info_file_path = output_dir_path.join(format!("{}.info.json", media.id));

        let mut thumn_is_downloaded = false;
        for (format, raw) in formats {
            let span = info_span!("iter", %format).entered();

            let format_id = &format.format_id;
            let max_file_size = self.yt_dlp_cfg.max_file_size;
            let media_file_path = output_dir_path.join(format!("{}.{audio_ext}", media.id));
            let info_file_path = &info_file_path;
            let executable_path = self.yt_dlp_cfg.executable_path.as_ref();
            let pot_provider_url = self.yt_pot_provider_cfg.url.as_ref();
            let timeout = self.timeouts_cfg.audio_download;
            let host = url.host();
            let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

            debug!("Downloading audio");
            let span = span.exit();

            fs::write(info_file_path, raw).map_err(Self::Err::InfoFile)?;

            if !thumn_is_downloaded {
                for thumb_url in media.get_thumb_urls(format.aspect_ration_kind()) {
                    match download_and_convert(thumb_url.as_str(), &thumb_file_path, "/usr/bin/ffmpeg", 5).await {
                        Ok(()) => {
                            let _guard = span.enter();
                            info!("Thumb downloaded");
                            thumn_is_downloaded = true;
                            break;
                        }
                        Err(err) => {
                            let _guard = span.enter();
                            warn!(%err, "Download thumb err");
                        }
                    }
                }
            }

            if let Err(err) = download_media(
                FormatStrategy::AudioOnly { audio_ext },
                format_id,
                max_file_size,
                output_dir_path,
                info_file_path,
                executable_path,
                pot_provider_url,
                timeout,
                cookie,
                progress_sender.as_ref(),
            )
            .await
            {
                let _guard = span.enter();
                error!(%err, "Download audio err");
                err_sender.send(err)?;
                continue;
            }
            let _guard = span.enter();
            info!("Audio downloaded");

            let media_in_fs = MediaInFS {
                path: media_file_path,
                thumb_path: if thumn_is_downloaded { Some(thumb_file_path) } else { None },
                temp_dir,
            };
            return Ok(Some((media_in_fs, format)));
        }
        Ok(None)
    }
}

pub struct DownloadVideoPlaylist {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
    pub cookies: Arc<Cookies>,
}

impl Interactor<DownloadMediaPlaylistInput<'_>> for &DownloadVideoPlaylist {
    type Output = ();
    type Err = DownloadMediaPlaylistErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaPlaylistInput {
            url,
            playlist,
            media_sender,
            errs_sender,
            progress_sender,
        }: DownloadMediaPlaylistInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let max_file_size = self.yt_dlp_cfg.max_file_size;
        let executable_path = self.yt_dlp_cfg.executable_path.as_ref();
        let pot_provider_url = self.yt_pot_provider_cfg.url.as_ref();
        let timeout = self.timeouts_cfg.video_download;
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        for (media, formats) in playlist {
            let temp_dir = TempDir::new().map_err(Self::Err::TempDir)?;
            let output_dir_path = temp_dir.path();
            let thumb_file_path = output_dir_path.join(format!("{}.jpg", media.id));
            let info_file_path = output_dir_path.join(format!("{}.info.json", media.id));

            let mut errs = vec![];
            let mut media_is_downloaded = false;
            let mut thumn_is_downloaded = false;
            for (format, raw) in formats {
                let span = info_span!("iter", id = media.id, %format).entered();

                let format_id = &format.format_id;
                let info_file_path = &info_file_path;
                let media_file_path = output_dir_path.join(format!("{}.{}", media.id, format.ext));

                debug!("Downloading video");
                let span = span.exit();

                fs::write(info_file_path, raw).map_err(Self::Err::InfoFile)?;

                if !thumn_is_downloaded {
                    for thumb_url in media.get_thumb_urls(format.aspect_ration_kind()) {
                        match download_and_convert(thumb_url.as_str(), &thumb_file_path, "/usr/bin/ffmpeg", 5).await {
                            Ok(()) => {
                                let _guard = span.enter();
                                info!("Thumb downloaded");
                                thumn_is_downloaded = true;
                                break;
                            }
                            Err(err) => {
                                let _guard = span.enter();
                                warn!(%err, "Download thumb err");
                            }
                        }
                    }
                }

                if let Err(err) = download_media(
                    FormatStrategy::VideoAndAudio,
                    format_id,
                    max_file_size,
                    output_dir_path,
                    info_file_path,
                    executable_path,
                    pot_provider_url,
                    timeout,
                    cookie,
                    progress_sender.as_ref(),
                )
                .await
                {
                    let _guard = span.enter();
                    error!(%err, "Download video err");
                    errs.push(err);
                    continue;
                }

                let _guard = span.enter();
                info!("Video downloaded");
                media_is_downloaded = true;

                let media_in_fs = MediaInFS {
                    path: media_file_path,
                    thumb_path: if thumn_is_downloaded { Some(thumb_file_path) } else { None },
                    temp_dir,
                };
                media_sender.send((media_in_fs, media, format))?;
                break;
            }
            if let Some(ref sender) = errs_sender {
                if !media_is_downloaded {
                    sender.send(errs)?;
                }
            }
        }

        Ok(())
    }
}

pub struct DownloadAudioPlaylist {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
    pub cookies: Arc<Cookies>,
}

impl Interactor<DownloadMediaPlaylistInput<'_>> for &DownloadAudioPlaylist {
    type Output = ();
    type Err = DownloadMediaPlaylistErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaPlaylistInput {
            url,
            playlist,
            media_sender,
            errs_sender,
            progress_sender,
        }: DownloadMediaPlaylistInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let max_file_size = self.yt_dlp_cfg.max_file_size;
        let executable_path = self.yt_dlp_cfg.executable_path.as_ref();
        let pot_provider_url = self.yt_pot_provider_cfg.url.as_ref();
        let timeout = self.timeouts_cfg.audio_download;
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());
        let audio_ext = "m4a";

        for (media, formats) in playlist {
            let temp_dir = TempDir::new().map_err(Self::Err::TempDir)?;
            let output_dir_path = temp_dir.path();
            let media_file_path = output_dir_path.join(format!("{}.{audio_ext}", media.id));
            let thumb_file_path = output_dir_path.join(format!("{}.jpg", media.id));
            let info_file_path = output_dir_path.join(format!("{}.info.json", media.id));

            let mut errs = vec![];
            let mut media_is_downloaded = false;
            let mut thumn_is_downloaded = false;
            for (format, raw) in formats {
                let span = info_span!("iter", id = media.id, %format).entered();

                let format_id = &format.format_id;
                let info_file_path = &info_file_path;

                debug!("Downloading audio");
                let span = span.exit();

                fs::write(info_file_path, raw).map_err(Self::Err::InfoFile)?;

                if !thumn_is_downloaded {
                    for thumb_url in media.get_thumb_urls(format.aspect_ration_kind()) {
                        match download_and_convert(thumb_url.as_str(), &thumb_file_path, "/usr/bin/ffmpeg", 5).await {
                            Ok(()) => {
                                let _guard = span.enter();
                                info!("Thumb downloaded");
                                thumn_is_downloaded = true;
                                break;
                            }
                            Err(err) => {
                                let _guard = span.enter();
                                warn!(%err, "Download thumb err");
                            }
                        }
                    }
                }

                if let Err(err) = download_media(
                    FormatStrategy::AudioOnly { audio_ext },
                    format_id,
                    max_file_size,
                    output_dir_path,
                    info_file_path,
                    executable_path,
                    pot_provider_url,
                    timeout,
                    cookie,
                    progress_sender.as_ref(),
                )
                .await
                {
                    let _guard = span.enter();
                    error!(%err, "Download audio err");
                    errs.push(err);
                    continue;
                }

                let _guard = span.enter();
                info!("Audio downloaded");
                media_is_downloaded = true;

                let media_in_fs = MediaInFS {
                    path: media_file_path,
                    thumb_path: if thumn_is_downloaded { Some(thumb_file_path) } else { None },
                    temp_dir,
                };
                media_sender.send((media_in_fs, media, format))?;
                break;
            }
            if let Some(ref sender) = errs_sender {
                if !media_is_downloaded {
                    sender.send(errs)?;
                }
            }
        }

        Ok(())
    }
}
