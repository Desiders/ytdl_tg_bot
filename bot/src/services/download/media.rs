use bytes::Bytes;
use futures_util::Stream;
use proto::downloader::{DownloadRequest, Section};
use std::{
    io,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing::instrument;
use url::Url;

use crate::{
    entities::{Media, MediaByteStream, MediaForUpload, MediaFormat, RawMediaWithFormat, Sections},
    interactors::Interactor,
    services::node_router::{download_media, DownloadErrorKind, DownloadEvent, DownloadSession, NodeRouter},
};

#[derive(thiserror::Error, Debug)]
pub enum DownloadMediaErrorKind {
    #[error("Temp dir error: {0}")]
    TempDir(io::Error),
    #[error("Channel error: {0}")]
    Channel(#[from] mpsc::error::SendError<DownloadErrorKind>),
    #[error(transparent)]
    Download(#[from] DownloadErrorKind),
}

#[derive(thiserror::Error, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum DownloadMediaPlaylistErrorKind {
    #[error("Temp dir error: {0}")]
    TempDir(io::Error),
    #[error("Channel error: {0}")]
    ErrChannel(#[from] mpsc::error::SendError<Vec<DownloadErrorKind>>),
    #[error("Channel error: {0}")]
    MediaChannel(#[from] mpsc::error::SendError<(MediaForUpload, Media, MediaFormat)>),
}

pub enum DownloadProgressEvent {
    Progress(String),
    Finished,
}

pub struct DownloadMediaInput<'a> {
    url: &'a Url,
    media: &'a Media,
    sections: Option<&'a Sections>,
    formats: Vec<(MediaFormat, RawMediaWithFormat)>,
    err_sender: mpsc::UnboundedSender<DownloadErrorKind>,
    progress_sender: Option<mpsc::UnboundedSender<DownloadProgressEvent>>,
}

impl<'a> DownloadMediaInput<'a> {
    pub fn new_with_progress(
        url: &'a Url,
        media: &'a Media,
        sections: Option<&'a Sections>,
        formats: Vec<(MediaFormat, RawMediaWithFormat)>,
    ) -> (
        Self,
        mpsc::UnboundedReceiver<DownloadErrorKind>,
        mpsc::UnboundedReceiver<DownloadProgressEvent>,
    ) {
        let (err_sender, err_receiver) = mpsc::unbounded_channel();
        let (progress_sender, progress_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                media,
                sections,
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
    sections: Option<&'a Sections>,
    media_sender: mpsc::UnboundedSender<(MediaForUpload, Media, MediaFormat)>,
    errs_sender: Option<mpsc::UnboundedSender<Vec<DownloadErrorKind>>>,
    progress_sender: Option<mpsc::UnboundedSender<DownloadProgressEvent>>,
}

impl<'a> DownloadMediaPlaylistInput<'a> {
    #[allow(clippy::type_complexity)]
    pub fn new_with_progress(
        url: &'a Url,
        playlist: Vec<(Media, Vec<(MediaFormat, RawMediaWithFormat)>)>,
        sections: Option<&'a Sections>,
    ) -> (
        Self,
        mpsc::UnboundedReceiver<(MediaForUpload, Media, MediaFormat)>,
        mpsc::UnboundedReceiver<Vec<DownloadErrorKind>>,
        mpsc::UnboundedReceiver<DownloadProgressEvent>,
    ) {
        let (media_sender, media_receiver) = mpsc::unbounded_channel();
        let (errs_sender, errs_receiver) = mpsc::unbounded_channel();
        let (progress_sender, progress_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                playlist,
                sections,
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
        sections: Option<&'a Sections>,
    ) -> (Self, mpsc::UnboundedReceiver<(MediaForUpload, Media, MediaFormat)>) {
        let (media_sender, media_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                playlist,
                sections,
                media_sender,
                errs_sender: None,
                progress_sender: None,
            },
            media_receiver,
        )
    }
}

pub struct DownloadVideo {
    pub node_router: Arc<NodeRouter>,
}

impl Interactor<DownloadMediaInput<'_>> for &DownloadVideo {
    type Output = Option<(MediaForUpload, MediaFormat)>;
    type Err = DownloadMediaErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaInput {
            url,
            media,
            sections,
            formats,
            err_sender,
            progress_sender,
        }: DownloadMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(Self::Err::TempDir)?;

        for (format, raw) in formats {
            let request = build_download_request(url, &format, raw, "video", "", sections, self.node_router.max_file_size());

            match download_with_retry(
                self.node_router.as_ref(),
                url.domain(),
                request,
                temp_dir.path(),
                &format,
                progress_sender.as_ref(),
            )
            .await
            {
                Ok(PreparedDownload {
                    path,
                    thumb_stream,
                    format,
                    stream,
                }) => {
                    let media_for_upload = MediaForUpload {
                        path,
                        thumb_stream,
                        temp_dir,
                        stream,
                    };
                    return Ok(Some((media_for_upload, format)));
                }
                Err(err) => {
                    err_sender.send(err)?;
                }
            }
        }

        let _ = media;
        Ok(None)
    }
}

pub struct DownloadAudio {
    pub node_router: Arc<NodeRouter>,
}

impl Interactor<DownloadMediaInput<'_>> for &DownloadAudio {
    type Output = Option<(MediaForUpload, MediaFormat)>;
    type Err = DownloadMediaErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaInput {
            url,
            media,
            sections,
            formats,
            err_sender,
            progress_sender,
        }: DownloadMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(Self::Err::TempDir)?;

        for (format, raw) in formats {
            let request = build_download_request(url, &format, raw, "audio", "m4a", sections, self.node_router.max_file_size());

            match download_with_retry(
                self.node_router.as_ref(),
                url.domain(),
                request,
                temp_dir.path(),
                &format,
                progress_sender.as_ref(),
            )
            .await
            {
                Ok(PreparedDownload {
                    path,
                    thumb_stream,
                    format,
                    stream,
                }) => {
                    let media_for_upload = MediaForUpload {
                        path,
                        thumb_stream,
                        temp_dir,
                        stream,
                    };
                    return Ok(Some((media_for_upload, format)));
                }
                Err(err) => {
                    err_sender.send(err)?;
                }
            }
        }

        let _ = media;
        Ok(None)
    }
}

pub struct DownloadVideoPlaylist {
    pub node_router: Arc<NodeRouter>,
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
            sections,
            media_sender,
            errs_sender,
            progress_sender,
        }: DownloadMediaPlaylistInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        for (media, formats) in playlist {
            let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(Self::Err::TempDir)?;
            let mut errs = vec![];
            let mut media_is_downloaded = false;

            for (format, raw) in formats {
                let request = build_download_request(url, &format, raw, "video", "", sections, self.node_router.max_file_size());

                match download_with_retry(
                    self.node_router.as_ref(),
                    url.domain(),
                    request,
                    temp_dir.path(),
                    &format,
                    progress_sender.as_ref(),
                )
                .await
                {
                    Ok(PreparedDownload {
                        path,
                        thumb_stream,
                        format,
                        stream,
                    }) => {
                        let media_for_upload = MediaForUpload {
                            path,
                            thumb_stream,
                            temp_dir,
                            stream,
                        };
                        media_sender.send((media_for_upload, media, format))?;
                        media_is_downloaded = true;
                        break;
                    }
                    Err(err) => {
                        errs.push(err);
                    }
                }
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
    pub node_router: Arc<NodeRouter>,
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
            sections,
            media_sender,
            errs_sender,
            progress_sender,
        }: DownloadMediaPlaylistInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        for (media, formats) in playlist {
            let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(Self::Err::TempDir)?;
            let mut errs = vec![];
            let mut media_is_downloaded = false;

            for (format, raw) in formats {
                let request = build_download_request(url, &format, raw, "audio", "m4a", sections, self.node_router.max_file_size());

                match download_with_retry(
                    self.node_router.as_ref(),
                    url.domain(),
                    request,
                    temp_dir.path(),
                    &format,
                    progress_sender.as_ref(),
                )
                .await
                {
                    Ok(PreparedDownload {
                        path,
                        thumb_stream,
                        format,
                        stream,
                    }) => {
                        let media_for_upload = MediaForUpload {
                            path,
                            thumb_stream,
                            temp_dir,
                            stream,
                        };
                        media_sender.send((media_for_upload, media, format))?;
                        media_is_downloaded = true;
                        break;
                    }
                    Err(err) => {
                        errs.push(err);
                    }
                }
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

struct PreparedDownload {
    path: PathBuf,
    thumb_stream: Option<MediaByteStream>,
    format: MediaFormat,
    stream: MediaByteStream,
}

async fn download_with_retry(
    node_router: &NodeRouter,
    domain: Option<&str>,
    request: DownloadRequest,
    output_dir: &Path,
    base_format: &MediaFormat,
    progress_sender: Option<&mpsc::UnboundedSender<DownloadProgressEvent>>,
) -> Result<PreparedDownload, DownloadErrorKind> {
    let session = download_media(node_router, domain, request).await?;
    build_downloaded_media(session, output_dir, base_format, progress_sender).await
}

async fn build_downloaded_media(
    session: DownloadSession,
    output_dir: &Path,
    base_format: &MediaFormat,
    progress_sender: Option<&mpsc::UnboundedSender<DownloadProgressEvent>>,
) -> Result<PreparedDownload, DownloadErrorKind> {
    let meta = session.meta().clone();
    let path = output_dir.join(format!("media.{}", meta.ext));
    let (media_sender, media_receiver) = mpsc::unbounded_channel();
    let (thumb_sender, thumb_receiver) = mpsc::unbounded_channel();
    let mut format = base_format.clone();
    format.ext = meta.ext;
    format.width = meta.width;
    format.height = meta.height;

    tokio::spawn(forward_download_stream(
        session,
        progress_sender.cloned(),
        media_sender,
        meta.has_thumbnail.then_some(thumb_sender),
    ));

    let stream = MediaByteStream::new(ChannelByteStream::new(media_receiver));
    let thumb_stream = meta
        .has_thumbnail
        .then(|| MediaByteStream::new(ChannelByteStream::new(thumb_receiver)));

    Ok(PreparedDownload {
        path,
        thumb_stream,
        format,
        stream,
    })
}

fn build_download_request(
    url: &Url,
    format: &MediaFormat,
    raw_info_json: String,
    media_type: &str,
    audio_ext: &str,
    sections: Option<&Sections>,
    max_file_size: u64,
) -> DownloadRequest {
    DownloadRequest {
        url: url.as_str().to_owned(),
        format_id: format.format_id.clone(),
        raw_info_json,
        media_type: media_type.to_owned(),
        audio_ext: audio_ext.to_owned(),
        section: sections.map(|sections| Section {
            start: sections.start,
            end: sections.end,
        }),
        max_file_size,
    }
}

struct ChannelByteStream {
    inner: Mutex<mpsc::UnboundedReceiver<Result<Bytes, io::Error>>>,
}

impl ChannelByteStream {
    fn new(receiver: mpsc::UnboundedReceiver<Result<Bytes, io::Error>>) -> Self {
        Self {
            inner: Mutex::new(receiver),
        }
    }
}

impl Stream for ChannelByteStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.lock().expect("Channel byte stream mutex poisoned").poll_recv(cx)
    }
}

async fn forward_download_stream(
    mut session: DownloadSession,
    progress_sender: Option<mpsc::UnboundedSender<DownloadProgressEvent>>,
    media_sender: mpsc::UnboundedSender<Result<Bytes, io::Error>>,
    thumb_sender: Option<mpsc::UnboundedSender<Result<Bytes, io::Error>>>,
) {
    loop {
        match session.next_event().await {
            Ok(Some(event)) => {
                if let Err(err) = handle_download_event(event, progress_sender.as_ref(), &media_sender, thumb_sender.as_ref()) {
                    let _ = media_sender.send(Err(err));
                    return;
                }
            }
            Ok(None) => {
                if let Some(sender) = progress_sender {
                    let _ = sender.send(DownloadProgressEvent::Finished);
                }
                return;
            }
            Err(err) => {
                let _ = media_sender.send(Err(io::Error::other(err)));
                return;
            }
        }
    }
}

fn handle_download_event(
    event: DownloadEvent,
    progress_sender: Option<&mpsc::UnboundedSender<DownloadProgressEvent>>,
    media_sender: &mpsc::UnboundedSender<Result<Bytes, io::Error>>,
    thumb_sender: Option<&mpsc::UnboundedSender<Result<Bytes, io::Error>>>,
) -> Result<(), io::Error> {
    match event {
        DownloadEvent::Progress(progress) => {
            if let Some(sender) = progress_sender {
                let _ = sender.send(DownloadProgressEvent::Progress(progress));
            }
            Ok(())
        }
        DownloadEvent::Data(data) => media_sender
            .send(Ok(data))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Media stream closed")),
        DownloadEvent::ThumbnailData(data) => thumb_sender
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Unexpected thumbnail stream"))?
            .send(Ok(data))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Thumbnail stream closed")),
    }
}
