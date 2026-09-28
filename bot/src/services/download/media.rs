use bytes::Bytes;
use futures_util::Stream;
use proto::downloader::{DownloadRequest, Section};
use std::{
    io,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tempfile::TempDir;
use tokio::{sync::mpsc, time::Instant};
use tracing::instrument;
use url::Url;

use crate::{
    entities::{Media, MediaByteStream, MediaForUpload, MediaFormat, RawMediaWithFormat, Sections},
    interactors::Interactor,
    services::node_router::{download_media, DownloadErrorKind, DownloadEvent, DownloadSession, NodeRouter},
};

const MEDIA_STREAM_CHANNEL_CAPACITY: usize = 4;

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
    #[error(transparent)]
    Download(#[from] DownloadErrorKind),
    #[error("Channel error: {0}")]
    ErrChannel(#[from] mpsc::error::SendError<Vec<DownloadErrorKind>>),
    #[error("Channel error: {0}")]
    MediaChannel(#[from] mpsc::error::SendError<(MediaForUpload, Media, MediaFormat, Option<i64>)>),
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
    media_sender: mpsc::UnboundedSender<(MediaForUpload, Media, MediaFormat, Option<i64>)>,
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
        mpsc::UnboundedReceiver<(MediaForUpload, Media, MediaFormat, Option<i64>)>,
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
    ) -> (Self, mpsc::UnboundedReceiver<(MediaForUpload, Media, MediaFormat, Option<i64>)>) {
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
    node_router: Arc<NodeRouter>,
}

impl DownloadVideo {
    #[must_use]
    pub const fn new(node_router: Arc<NodeRouter>) -> Self {
        Self { node_router }
    }
}

impl Interactor<DownloadMediaInput<'_>> for &DownloadVideo {
    type Output = Option<(MediaForUpload, MediaFormat, Option<i64>)>;
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

        let mut deadline = Instant::now() + Duration::from_secs(proto::MEDIA_EXECUTION_LIMIT_SECS);
        for (format, raw) in formats {
            let request = DownloadRequest {
                url: url.as_str().to_owned(),
                format_id: format.format_id.clone(),
                raw_info_json: raw,
                media_type: "video".to_owned(),
                audio_ext: String::new(),
                section: sections.map(|sections| Section {
                    start: sections.start,
                    end: sections.end,
                }),
                max_file_size: self.node_router.max_file_size(),
            };

            match prepare_download(
                self.node_router.as_ref(),
                url.domain(),
                request,
                temp_dir.path(),
                &format,
                progress_sender.as_ref(),
                &mut deadline,
            )
            .await
            {
                Ok(PreparedDownload {
                    path,
                    thumb_stream,
                    format,
                    duration,
                    stream,
                }) => {
                    let media_for_upload = MediaForUpload {
                        path,
                        thumb_stream,
                        temp_dir,
                        stream,
                        deadline,
                    };
                    return Ok(Some((media_for_upload, format, duration)));
                }
                Err(err) => {
                    if err.is_execution_uncertain() {
                        return Err(err.into());
                    }
                    err_sender.send(err)?;
                }
            }
        }

        let _ = media;
        Ok(None)
    }
}

pub struct DownloadAudio {
    node_router: Arc<NodeRouter>,
}

impl DownloadAudio {
    #[must_use]
    pub const fn new(node_router: Arc<NodeRouter>) -> Self {
        Self { node_router }
    }
}

impl Interactor<DownloadMediaInput<'_>> for &DownloadAudio {
    type Output = Option<(MediaForUpload, MediaFormat, Option<i64>)>;
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

        let mut deadline = Instant::now() + Duration::from_secs(proto::MEDIA_EXECUTION_LIMIT_SECS);
        for (format, raw) in formats {
            let request = DownloadRequest {
                url: url.as_str().to_owned(),
                format_id: format.format_id.clone(),
                raw_info_json: raw,
                media_type: "audio".to_owned(),
                audio_ext: "m4a".to_owned(),
                section: sections.map(|sections| Section {
                    start: sections.start,
                    end: sections.end,
                }),
                max_file_size: self.node_router.max_file_size(),
            };

            match prepare_download(
                self.node_router.as_ref(),
                url.domain(),
                request,
                temp_dir.path(),
                &format,
                progress_sender.as_ref(),
                &mut deadline,
            )
            .await
            {
                Ok(PreparedDownload {
                    path,
                    thumb_stream,
                    format,
                    duration,
                    stream,
                }) => {
                    let media_for_upload = MediaForUpload {
                        path,
                        thumb_stream,
                        temp_dir,
                        stream,
                        deadline,
                    };
                    return Ok(Some((media_for_upload, format, duration)));
                }
                Err(err) => {
                    if err.is_execution_uncertain() {
                        return Err(err.into());
                    }
                    err_sender.send(err)?;
                }
            }
        }

        let _ = media;
        Ok(None)
    }
}

pub struct DownloadPhoto {
    node_router: Arc<NodeRouter>,
}

impl DownloadPhoto {
    #[must_use]
    pub const fn new(node_router: Arc<NodeRouter>) -> Self {
        Self { node_router }
    }
}

impl Interactor<DownloadMediaInput<'_>> for &DownloadPhoto {
    type Output = Option<(MediaForUpload, MediaFormat, Option<i64>)>;
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

        let mut deadline = Instant::now() + Duration::from_secs(proto::MEDIA_EXECUTION_LIMIT_SECS);
        for (format, raw) in formats {
            let request = DownloadRequest {
                url: url.as_str().to_owned(),
                format_id: format.format_id.clone(),
                raw_info_json: raw,
                media_type: "photo".to_owned(),
                audio_ext: String::new(),
                section: sections.map(|sections| Section {
                    start: sections.start,
                    end: sections.end,
                }),
                max_file_size: self.node_router.max_file_size(),
            };

            match prepare_download(
                self.node_router.as_ref(),
                url.domain(),
                request,
                temp_dir.path(),
                &format,
                progress_sender.as_ref(),
                &mut deadline,
            )
            .await
            {
                Ok(PreparedDownload {
                    path,
                    thumb_stream,
                    format,
                    duration,
                    stream,
                }) => {
                    let media_for_upload = MediaForUpload {
                        path,
                        thumb_stream,
                        temp_dir,
                        stream,
                        deadline,
                    };
                    return Ok(Some((media_for_upload, format, duration)));
                }
                Err(err) => {
                    if err.is_execution_uncertain() {
                        return Err(err.into());
                    }
                    err_sender.send(err)?;
                }
            }
        }

        let _ = media;
        Ok(None)
    }
}

pub struct DownloadVideoPlaylist {
    node_router: Arc<NodeRouter>,
}

impl DownloadVideoPlaylist {
    #[must_use]
    pub const fn new(node_router: Arc<NodeRouter>) -> Self {
        Self { node_router }
    }
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

            let mut deadline = Instant::now() + Duration::from_secs(proto::MEDIA_EXECUTION_LIMIT_SECS);
            for (format, raw) in formats {
                let request = DownloadRequest {
                    url: url.as_str().to_owned(),
                    format_id: format.format_id.clone(),
                    raw_info_json: raw,
                    media_type: "video".to_owned(),
                    audio_ext: String::new(),
                    section: sections.map(|sections| Section {
                        start: sections.start,
                        end: sections.end,
                    }),
                    max_file_size: self.node_router.max_file_size(),
                };

                match prepare_download(
                    self.node_router.as_ref(),
                    url.domain(),
                    request,
                    temp_dir.path(),
                    &format,
                    progress_sender.as_ref(),
                    &mut deadline,
                )
                .await
                {
                    Ok(PreparedDownload {
                        path,
                        thumb_stream,
                        format,
                        duration,
                        stream,
                    }) => {
                        let media_for_upload = MediaForUpload {
                            path,
                            thumb_stream,
                            temp_dir,
                            stream,
                            deadline,
                        };
                        media_sender.send((media_for_upload, media, format, duration))?;
                        media_is_downloaded = true;
                        break;
                    }
                    Err(err) => {
                        if err.is_execution_uncertain() {
                            return Err(err.into());
                        }
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
    node_router: Arc<NodeRouter>,
}

impl DownloadAudioPlaylist {
    #[must_use]
    pub const fn new(node_router: Arc<NodeRouter>) -> Self {
        Self { node_router }
    }
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

            let mut deadline = Instant::now() + Duration::from_secs(proto::MEDIA_EXECUTION_LIMIT_SECS);
            for (format, raw) in formats {
                let request = DownloadRequest {
                    url: url.as_str().to_owned(),
                    format_id: format.format_id.clone(),
                    raw_info_json: raw,
                    media_type: "audio".to_owned(),
                    audio_ext: "m4a".to_owned(),
                    section: sections.map(|sections| Section {
                        start: sections.start,
                        end: sections.end,
                    }),
                    max_file_size: self.node_router.max_file_size(),
                };

                match prepare_download(
                    self.node_router.as_ref(),
                    url.domain(),
                    request,
                    temp_dir.path(),
                    &format,
                    progress_sender.as_ref(),
                    &mut deadline,
                )
                .await
                {
                    Ok(PreparedDownload {
                        path,
                        thumb_stream,
                        format,
                        duration,
                        stream,
                    }) => {
                        let media_for_upload = MediaForUpload {
                            path,
                            thumb_stream,
                            temp_dir,
                            stream,
                            deadline,
                        };
                        media_sender.send((media_for_upload, media, format, duration))?;
                        media_is_downloaded = true;
                        break;
                    }
                    Err(err) => {
                        if err.is_execution_uncertain() {
                            return Err(err.into());
                        }
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

pub struct DownloadPhotoPlaylist {
    node_router: Arc<NodeRouter>,
}

impl DownloadPhotoPlaylist {
    #[must_use]
    pub const fn new(node_router: Arc<NodeRouter>) -> Self {
        Self { node_router }
    }
}

impl Interactor<DownloadMediaPlaylistInput<'_>> for &DownloadPhotoPlaylist {
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

            let mut deadline = Instant::now() + Duration::from_secs(proto::MEDIA_EXECUTION_LIMIT_SECS);
            for (format, raw) in formats {
                let request = DownloadRequest {
                    url: url.as_str().to_owned(),
                    format_id: format.format_id.clone(),
                    raw_info_json: raw,
                    media_type: "photo".to_owned(),
                    audio_ext: String::new(),
                    section: sections.map(|sections| Section {
                        start: sections.start,
                        end: sections.end,
                    }),
                    max_file_size: self.node_router.max_file_size(),
                };

                match prepare_download(
                    self.node_router.as_ref(),
                    url.domain(),
                    request,
                    temp_dir.path(),
                    &format,
                    progress_sender.as_ref(),
                    &mut deadline,
                )
                .await
                {
                    Ok(PreparedDownload {
                        path,
                        thumb_stream,
                        format,
                        duration,
                        stream,
                    }) => {
                        let media_for_upload = MediaForUpload {
                            path,
                            thumb_stream,
                            temp_dir,
                            stream,
                            deadline,
                        };
                        media_sender.send((media_for_upload, media, format, duration))?;
                        media_is_downloaded = true;
                        break;
                    }
                    Err(err) => {
                        if err.is_execution_uncertain() {
                            return Err(err.into());
                        }
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
    duration: Option<i64>,
    stream: MediaByteStream,
}

async fn prepare_download(
    node_router: &NodeRouter,
    domain: Option<&str>,
    request: DownloadRequest,
    output_dir: &Path,
    base_format: &MediaFormat,
    progress_sender: Option<&mpsc::UnboundedSender<DownloadProgressEvent>>,
    deadline: &mut Instant,
) -> Result<PreparedDownload, DownloadErrorKind> {
    if Instant::now() >= *deadline {
        // Earlier format attempts have ended; no new RPC may begin after the budget.
        return Err(io::Error::new(io::ErrorKind::TimedOut, "Media execution limit exceeded").into());
    }
    // The node streams progress while downloading and only sends `Meta` once the download
    // succeeds, so this resolves only after a usable file exists. Forward the pre-`Meta`
    // progress to the UI; a download failure returns here as an error instead of breaking
    // the upload that would otherwise have already started.
    let on_progress = |progress: String| {
        if let Some(sender) = progress_sender {
            let _ = sender.send(DownloadProgressEvent::Progress(progress));
        }
    };
    let session = download_media(node_router, domain, request, on_progress, deadline).await?;
    Ok(build_downloaded_media(session, output_dir, base_format, progress_sender))
}

fn build_downloaded_media(
    session: DownloadSession,
    output_dir: &Path,
    base_format: &MediaFormat,
    progress_sender: Option<&mpsc::UnboundedSender<DownloadProgressEvent>>,
) -> PreparedDownload {
    let meta = session.meta().clone();
    let path = output_dir.join(format!("media.{}", meta.ext));
    let (media_sender, media_receiver) = mpsc::channel(MEDIA_STREAM_CHANNEL_CAPACITY);
    let (thumb_sender, thumb_receiver) = mpsc::unbounded_channel();
    let mut format = base_format.clone();
    format.ext = meta.ext;
    format.width = meta.width;
    format.height = meta.height;

    tokio::spawn(forward_download_stream(
        session,
        media_sender,
        meta.has_thumbnail.then_some(thumb_sender),
    ));
    // The upload starts after progress collection finishes. Do not keep its sender
    // alive in the byte forwarder, which can block until that upload consumes bytes.
    if let Some(sender) = progress_sender {
        let _ = sender.send(DownloadProgressEvent::Finished);
    }

    let stream = MediaByteStream::new(ChannelByteStream::new(media_receiver));
    let thumb_stream = meta
        .has_thumbnail
        .then(|| MediaByteStream::new(ChannelByteStream::new(thumb_receiver)));

    PreparedDownload {
        path,
        thumb_stream,
        format,
        duration: meta.duration,
        stream,
    }
}

trait PollBytes: Send {
    fn poll_bytes(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Bytes, io::Error>>>;
}

impl PollBytes for mpsc::UnboundedReceiver<Result<Bytes, io::Error>> {
    fn poll_bytes(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Bytes, io::Error>>> {
        self.poll_recv(cx)
    }
}

impl PollBytes for mpsc::Receiver<Result<Bytes, io::Error>> {
    fn poll_bytes(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Bytes, io::Error>>> {
        self.poll_recv(cx)
    }
}

struct ChannelByteStream<R: PollBytes> {
    inner: Mutex<R>,
}

impl<R: PollBytes> ChannelByteStream<R> {
    fn new(receiver: R) -> Self {
        Self {
            inner: Mutex::new(receiver),
        }
    }
}

impl<R: PollBytes> Stream for ChannelByteStream<R> {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.lock().expect("Channel byte stream mutex poisoned").poll_bytes(cx)
    }
}

async fn forward_download_stream(
    mut session: DownloadSession,
    media_sender: mpsc::Sender<Result<Bytes, io::Error>>,
    mut thumb_sender: Option<mpsc::UnboundedSender<Result<Bytes, io::Error>>>,
) {
    loop {
        let event = tokio::select! {
            biased;
            () = media_sender.closed() => return,
            event = session.next_event() => event,
        };
        match event {
            Ok(Some(event)) => {
                if matches!(&event, DownloadEvent::Data(_)) {
                    // Thumbnail bytes precede media bytes; close that part before
                    // backpressure can block the media stream.
                    thumb_sender.take();
                }
                if let Err(err) = handle_download_event(event, &media_sender, thumb_sender.as_ref()).await {
                    let _ = media_sender.send(Err(err)).await;
                    return;
                }
            }
            Ok(None) => return,
            Err(err) => {
                let _ = media_sender.send(Err(io::Error::other(err))).await;
                return;
            }
        }
    }
}

async fn handle_download_event(
    event: DownloadEvent,
    media_sender: &mpsc::Sender<Result<Bytes, io::Error>>,
    thumb_sender: Option<&mpsc::UnboundedSender<Result<Bytes, io::Error>>>,
) -> Result<(), io::Error> {
    match event {
        DownloadEvent::Progress(_) => Ok(()),
        DownloadEvent::Data(data) => media_sender
            .send(Ok(data))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Media stream closed")),
        DownloadEvent::ThumbnailData(data) => thumb_sender
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Unexpected thumbnail stream"))?
            .send(Ok(data))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Thumbnail stream closed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn post_meta_progress_does_not_wait_for_an_upload_consumer() {
        let (sender, _receiver) = mpsc::channel(1);
        sender.send(Ok(Bytes::from_static(b"media"))).await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            handle_download_event(DownloadEvent::Progress("Synthetic progress".into()), &sender, None),
        )
        .await
        .unwrap()
        .unwrap();
    }

    #[tokio::test]
    async fn media_forwarding_waits_for_capacity_without_buffering_more_bytes() {
        let (sender, mut receiver) = mpsc::channel(1);
        sender.send(Ok(Bytes::from_static(b"first"))).await.unwrap();
        let forwarding = handle_download_event(DownloadEvent::Data(Bytes::from_static(b"second")), &sender, None);
        tokio::pin!(forwarding);
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut forwarding).await.is_err());
        assert_eq!(receiver.recv().await.unwrap().unwrap(), Bytes::from_static(b"first"));
        forwarding.await.unwrap();
        assert_eq!(receiver.recv().await.unwrap().unwrap(), Bytes::from_static(b"second"));
    }
}
