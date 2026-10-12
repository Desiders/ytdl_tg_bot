use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc, Mutex,
    },
    time::Duration,
};

use downloader_client::{outcome::ExecutionOutcome, DownloadErrorKind, NodeRouter};
use futures_util::{stream, Stream, StreamExt as _};
use proto::downloader::{
    download_chunk::Payload,
    downloader_server::{Downloader, DownloaderServer},
    node_capabilities_server::{NodeCapabilities, NodeCapabilitiesServer},
    DownloadChunk, DownloadMeta, DownloadRequest, Empty, MediaInfoRequest, MediaInfoResponse, NodeStatus, PostEntry,
    SupportedDomainsResponse,
};
use sea_orm::DatabaseConnection;
use tokio::{
    net::TcpListener,
    sync::{watch, Notify},
    task::JoinHandle,
    time::Instant,
};
use tonic::{
    transport::{Identity, Server, ServerTlsConfig},
    Request, Response, Status,
};
use url::Url;

use super::{
    audio,
    auto::{AudioFulfiller, FulfillCtx},
    chosen_inline, photo, post, test_support, video, Interactor,
};
use crate::{
    config::Config,
    database::{SeaOrmTxManager, TxManager, TxManagerFactories},
    entities::{language::Language, ChatConfig, DownloadJob, JobTarget, Media, MediaForUpload, MediaFormat, MediaInPlaylist, Params},
    services::{
        download::media,
        downloaded_media,
        get_media::{self, GetMediaByURLKind},
        messenger::{
            AnswerCallbackRequest, AnswerInlineErrorRequest, AnswerInlineQueryRequest, DeleteMessageRequest, EditCaptionRequest,
            EditMediaByIdRequest, EditMenuRequest, EditTarget, EditTextRequest, MessengerError, MessengerPort, SendMediaByIdRequest,
            SendMediaGroupRequest, SendMenuRequest, SendTextRequest, SentMessage, TextFormat, UploadAudioRequest, UploadPhotoRequest,
            UploadPhotoUrlRequest, UploadVideoRequest,
        },
        send_media,
    },
    utils::{ErrorFormatter, UrlCleaner},
    value_objects::MediaType,
};

#[derive(Default)]
struct State {
    events: Mutex<Vec<String>>,
    replies: Mutex<VecDeque<Option<Status>>>,
    post_replies: Mutex<VecDeque<Result<MediaInfoResponse, Status>>>,
    post_edit_error: Mutex<Option<MessengerError>>,
    post_send_error: Mutex<Option<MessengerError>>,
    post_metadata_active: AtomicBool,
    edits: Mutex<Vec<String>>,
    media_edits: Mutex<Vec<String>>,
    optional_edit_started: Notify,
    block_optional_edits: bool,
    reject_upload: bool,
    reject_media_edit: bool,
}

impl State {
    fn event(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }

    fn post_edit(&self, target: &EditTarget<'_>, text: &str, as_caption: bool) -> Result<(), MessengerError> {
        if let Some(error) = self.post_edit_error.lock().unwrap().take() {
            self.event("post edit rejected");
            return Err(error);
        }
        let target = match target {
            EditTarget::ChatMessage { chat_id, message_id } => format!("chat:{chat_id}:{message_id}"),
            EditTarget::InlineMessage { inline_message_id } => format!("inline:{inline_message_id}"),
        };
        self.event(format!("post edit {target} caption={as_caption}: {text}"));
        self.edits.lock().unwrap().push(text.to_owned());
        Ok(())
    }
}

struct FakeNode(Arc<State>);

#[tonic::async_trait]
impl Downloader for FakeNode {
    type DownloadMediaStream = Pin<Box<dyn Stream<Item = Result<DownloadChunk, Status>> + Send>>;

    async fn get_media_info(&self, request: Request<MediaInfoRequest>) -> Result<Response<MediaInfoResponse>, Status> {
        assert_eq!(request.into_inner().media_type, "post", "Media tests use prefetched metadata");
        assert!(self
            .0
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event.as_str(), "final command send" | "final command edit" | "final inline edit")));
        self.0.post_metadata_active.store(true, Relaxed);
        self.0.event("post metadata");
        self.0
            .post_replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("Expected post metadata request")
            .map(Response::new)
    }

    async fn download_media(&self, request: Request<DownloadRequest>) -> Result<Response<Self::DownloadMediaStream>, Status> {
        self.0.event(format!("download {}", request.into_inner().url));
        let error = self.0.replies.lock().unwrap().pop_front().flatten();
        if let Some(error) = error {
            return Ok(Response::new(Box::pin(stream::iter([Err(error)]))));
        }
        let state = self.0.clone();
        let chunks = stream::unfold(0, move |index| {
            let state = state.clone();
            async move {
                let payload = match index {
                    0 => Payload::Progress("50%".into()),
                    1 => {
                        if state.block_optional_edits {
                            state.optional_edit_started.notified().await;
                        }
                        Payload::Meta(DownloadMeta {
                            ext: "mp4".into(),
                            ..Default::default()
                        })
                    }
                    2..=9 => Payload::Data(vec![1; 1024]),
                    10 => Payload::Complete(true),
                    _ => return None,
                };
                Some((Ok(DownloadChunk { payload: Some(payload) }), index + 1))
            }
        });
        Ok(Response::new(Box::pin(chunks)))
    }
}

#[tonic::async_trait]
impl NodeCapabilities for FakeNode {
    async fn get_status(&self, _: Request<Empty>) -> Result<Response<NodeStatus>, Status> {
        Ok(Response::new(NodeStatus {
            active_downloads: 0,
            max_concurrent: 5,
        }))
    }
    async fn get_supported_domains(&self, _: Request<Empty>) -> Result<Response<SupportedDomainsResponse>, Status> {
        Ok(Response::new(SupportedDomainsResponse::default()))
    }
}

struct RecordingMessenger(Arc<State>);

impl RecordingMessenger {
    fn edit_media(&self, kind: &str, request: &EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        if self.0.reject_media_edit {
            self.0.event("media edit rejected");
            return Err(MessengerError::new("Synthetic media edit failure"));
        }
        match request.target {
            EditTarget::ChatMessage { chat_id, message_id } => {
                self.0.media_edits.lock().unwrap().push(format!(
                    "{kind} chat:{chat_id}:{message_id} id={} caption={:?}",
                    request.remote_id, request.caption
                ));
                self.0.event("final command edit");
            }
            EditTarget::InlineMessage { inline_message_id } => {
                assert_eq!(inline_message_id, "synthetic-inline");
                self.0.event("final inline edit");
            }
        }
        Ok(())
    }

    async fn upload(&self, media: MediaForUpload) -> Result<Box<str>, MessengerError> {
        self.0.event("upload started");
        // A later item must not already have spent time waiting behind this upload.
        assert!(media.deadline.duration_since(Instant::now()) > Duration::from_secs(350));
        let mut bytes = media.stream.into_inner();
        let mut total = 0;
        while let Some(chunk) = bytes.next().await {
            total += chunk.unwrap().len();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(total, 8 * 1024);
        self.0.event("upload finished");
        if self.0.reject_upload {
            return Err(MessengerError::new("Too Many Requests: retry after 2"));
        }
        Ok("synthetic-file-id".into())
    }
}

struct OptionalEdit<'a>(&'a State);
impl Drop for OptionalEdit<'_> {
    fn drop(&mut self) {
        self.0.event("optional edit cancelled");
    }
}

impl MessengerPort for RecordingMessenger {
    async fn username(&self) -> Result<String, MessengerError> {
        Ok("test_bot".into())
    }
    async fn send_text(&self, request: SendTextRequest<'_>) -> Result<SentMessage, MessengerError> {
        if self.0.post_metadata_active.load(Relaxed) {
            assert_eq!(request.format, Some(TextFormat::Html));
            assert!(request.disable_link_preview);
            if let Some(error) = self.0.post_send_error.lock().unwrap().take() {
                self.0.event("post send rejected");
                return Err(error);
            }
            self.0.event(format!(
                "post text {} reply={:?}: {}",
                request.chat_id, request.reply_to_message_id, request.text
            ));
        }
        Ok(SentMessage { message_id: 1 })
    }
    async fn edit_text(&self, request: EditTextRequest<'_>) -> Result<(), MessengerError> {
        if self.0.post_metadata_active.load(Relaxed) && !request.is_progress {
            assert_eq!(request.format, Some(TextFormat::Html));
            assert!(request.disable_link_preview);
            assert!(request.clear_inline_keyboard);
            return self.0.post_edit(&request.target, request.text, false);
        }
        if self.0.block_optional_edits && (request.is_progress || request.text.contains("Sending")) {
            let _edit = OptionalEdit(&self.0);
            self.0.optional_edit_started.notify_one();
            std::future::pending::<()>().await;
        }
        if request.text.contains("Sending") {
            self.0.event("sending");
        }
        if !request.is_progress {
            self.0.edits.lock().unwrap().push(request.text.to_owned());
        }
        Ok(())
    }
    async fn edit_caption(&self, request: EditCaptionRequest<'_>) -> Result<(), MessengerError> {
        assert!(self.0.post_metadata_active.load(Relaxed));
        assert_eq!(request.format, Some(TextFormat::Html));
        assert!(request.clear_inline_keyboard);
        self.0.post_edit(&request.target, request.text, true)
    }
    async fn delete_message(&self, _: DeleteMessageRequest) -> Result<(), MessengerError> {
        self.0.event("delete");
        Ok(())
    }
    async fn upload_video(&self, request: UploadVideoRequest<'_>) -> Result<Box<str>, MessengerError> {
        self.upload(request.media_for_upload).await
    }
    async fn upload_audio(&self, request: UploadAudioRequest<'_>) -> Result<Box<str>, MessengerError> {
        self.upload(request.media_for_upload).await
    }
    async fn send_video_by_id(&self, _: SendMediaByIdRequest<'_>) -> Result<SentMessage, MessengerError> {
        self.0.event("final command send");
        Ok(SentMessage { message_id: 42 })
    }
    async fn send_audio_by_id(&self, _: SendMediaByIdRequest<'_>) -> Result<SentMessage, MessengerError> {
        self.0.event("final command send");
        Ok(SentMessage { message_id: 42 })
    }
    async fn send_video_group(&self, request: SendMediaGroupRequest) -> Result<Option<SentMessage>, MessengerError> {
        if request.items.is_empty() {
            return Ok(None);
        }
        self.0.event("final command send");
        Ok(Some(SentMessage { message_id: 42 }))
    }
    async fn send_audio_group(&self, request: SendMediaGroupRequest) -> Result<Option<SentMessage>, MessengerError> {
        if request.items.is_empty() {
            return Ok(None);
        }
        self.0.event("final command send");
        Ok(Some(SentMessage { message_id: 42 }))
    }
    async fn edit_video_by_id(&self, request: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        self.edit_media("video", &request)
    }
    async fn edit_audio_by_id(&self, request: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        self.edit_media("audio", &request)
    }
    async fn send_menu(&self, _: SendMenuRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn edit_menu(&self, _: EditMenuRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn answer_callback(&self, _: AnswerCallbackRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn answer_inline_error(&self, _: AnswerInlineErrorRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn answer_inline_query(&self, _: AnswerInlineQueryRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn upload_photo(&self, _: UploadPhotoRequest<'_>) -> Result<Box<str>, MessengerError> {
        unreachable!()
    }
    async fn upload_photo_url(&self, request: UploadPhotoUrlRequest<'_>) -> Result<Box<str>, MessengerError> {
        assert_eq!(request.photo_url.host_str(), Some("example.test"));
        self.0.event("photo upload");
        Ok("synthetic-file-id".into())
    }
    async fn send_photo_by_id(&self, _: SendMediaByIdRequest<'_>) -> Result<SentMessage, MessengerError> {
        self.0.event("final command send");
        Ok(SentMessage { message_id: 42 })
    }
    async fn edit_photo_by_id(&self, request: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        self.edit_media("photo", &request)
    }
    async fn send_photo_group(&self, request: SendMediaGroupRequest) -> Result<Option<SentMessage>, MessengerError> {
        if request.items.is_empty() {
            return Ok(None);
        }
        self.0.event("final command send");
        Ok(Some(SentMessage { message_id: 42 }))
    }
}

struct Fixture {
    state: Arc<State>,
    router: Arc<NodeRouter>,
    messenger: Arc<RecordingMessenger>,
    cfg: Arc<Config>,
    formatter: Arc<ErrorFormatter>,
    tx: Arc<Box<dyn TxManager>>,
    cleaner: Arc<UrlCleaner>,
    server: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new(state: State) -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let state = Arc::new(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let incoming = stream::unfold(listener, |listener| async {
            Some((listener.accept().await.map(|(socket, _)| socket), listener))
        });
        let server = Server::builder()
            .tls_config(ServerTlsConfig::new().identity(Identity::from_pem(
                include_bytes!("test_data/cert.pem"),
                include_bytes!("test_data/key.pem"),
            )))
            .unwrap()
            .add_service(DownloaderServer::new(FakeNode(state.clone())))
            .add_service(NodeCapabilitiesServer::new(FakeNode(state.clone())));
        let server = tokio::spawn(async move {
            server.serve_with_incoming(incoming).await.unwrap();
        });
        let router = test_support::router(port);
        router.refresh_status().await;
        assert_eq!(router.subscribe_capacity().borrow().available, 5);
        let cfg = test_support::config();
        Self {
            router,
            server,
            messenger: Arc::new(RecordingMessenger(state.clone())),
            state,
            cleaner: Arc::new(UrlCleaner::from_embedded_rules(&cfg.tracking_params).unwrap()),
            cfg: Arc::new(cfg),
            formatter: Arc::new(ErrorFormatter::new("test")),
            // Metadata is prefetched; cache writes may fail without affecting delivery.
            tx: Arc::new(Box::new(SeaOrmTxManager::new(
                Arc::new(DatabaseConnection::default()),
                Arc::new(TxManagerFactories::default()),
            ))),
        }
    }

    async fn command(&self, audio: bool, count: i16) -> Option<SentMessage> {
        Box::pin(self.command_media(if audio { MediaType::Audio } else { MediaType::Video }, playlist(count), None)).await
    }

    async fn command_media(
        &self,
        media_type: MediaType,
        prefetched: GetMediaByURLKind,
        progress: Option<(i64, &str)>,
    ) -> Option<SentMessage> {
        self.state.post_metadata_active.store(false, Relaxed);
        let url = Url::parse("https://example.test/playlist").unwrap();
        let params = Params::default();
        let chat = ChatConfig::new(1, false, "en".into());
        let current = watch::channel(None).0;
        if matches!(media_type, MediaType::Audio) {
            let interactor = audio::Download::new(
                self.cfg.clone(),
                self.formatter.clone(),
                self.messenger.clone(),
                Arc::new(get_media::GetAudioByURL::new(
                    self.router.clone(),
                    self.cleaner.clone(),
                    self.tx.clone(),
                )),
                Arc::new(media::DownloadAudio::new(self.router.clone())),
                Arc::new(send_media::upload::SendAudio::new(self.messenger.clone())),
                Arc::new(send_media::id::SendAudio::new(self.messenger.clone())),
                Arc::new(send_media::id::SendAudioPlaylist::new(self.messenger.clone())),
                Arc::new(downloaded_media::AddAudio::new(self.tx.clone())),
            );
            interactor
                .execute(audio::DownloadInput {
                    current_message: &current,
                    message_id: 1,
                    chat_id: 1,
                    params: &params,
                    url: &url,
                    chat_cfg: &chat,
                    link_is_visible: false,
                    prefetched: Some(prefetched),
                    base_text: progress.map(|(_, text)| text),
                    progress_message_id: progress.map(|(message_id, _)| message_id),
                })
                .await
                .unwrap()
        } else if matches!(media_type, MediaType::Video) {
            let interactor = video::Download::new(
                self.cfg.clone(),
                self.formatter.clone(),
                self.messenger.clone(),
                Arc::new(get_media::GetVideoByURL::new(
                    self.router.clone(),
                    self.cleaner.clone(),
                    self.tx.clone(),
                )),
                Arc::new(media::DownloadVideo::new(self.router.clone())),
                Arc::new(send_media::upload::SendVideo::new(self.messenger.clone())),
                Arc::new(send_media::id::SendVideo::new(self.messenger.clone())),
                Arc::new(send_media::id::SendVideoPlaylist::new(self.messenger.clone())),
                Arc::new(downloaded_media::AddVideo::new(self.tx.clone())),
            );
            interactor
                .execute(video::DownloadInput {
                    current_message: &current,
                    message_id: 1,
                    chat_id: 1,
                    params: &params,
                    url: &url,
                    chat_cfg: &chat,
                    link_is_visible: false,
                    prefetched: Some(prefetched),
                })
                .await
                .unwrap()
        } else {
            photo::Download::new(
                self.cfg.clone(),
                self.formatter.clone(),
                self.messenger.clone(),
                Arc::new(get_media::GetPhotoByURL::new(
                    self.router.clone(),
                    self.cleaner.clone(),
                    self.tx.clone(),
                )),
                Arc::new(send_media::upload::SendPhotoUrl::new(self.messenger.clone())),
                Arc::new(send_media::id::SendPhoto::new(self.messenger.clone())),
                Arc::new(send_media::id::SendPhotoPlaylist::new(self.messenger.clone())),
                Arc::new(downloaded_media::AddPhoto::new(self.tx.clone())),
            )
            .execute(photo::DownloadInput {
                current_message: &current,
                message_id: 1,
                chat_id: 1,
                params: &params,
                url: &url,
                chat_cfg: &chat,
                link_is_visible: false,
                prefetched: Some(prefetched),
            })
            .await
            .unwrap()
        }
    }

    async fn inline(&self, audio: bool) -> bool {
        self.state.post_metadata_active.store(false, Relaxed);
        let url = Url::parse("https://example.test/playlist").unwrap();
        let params = Params::default();
        let chat = ChatConfig::new(1, false, "en".into());
        let input = chosen_inline::DownloadInput {
            params: &params,
            url: Some(&url),
            chat_cfg: &chat,
            link_is_visible: false,
            inline_message_id: "synthetic-inline",
            result_id: "synthetic-result",
            prefetched: Some(playlist(1)),
        };
        if audio {
            chosen_inline::DownloadAudio::new(
                self.cfg.clone(),
                self.formatter.clone(),
                self.messenger.clone(),
                Arc::new(get_media::GetAudioByURL::new(
                    self.router.clone(),
                    self.cleaner.clone(),
                    self.tx.clone(),
                )),
                Arc::new(media::DownloadAudio::new(self.router.clone())),
                Arc::new(send_media::upload::SendAudio::new(self.messenger.clone())),
                Arc::new(send_media::id::EditAudio::new(self.messenger.clone())),
                Arc::new(downloaded_media::AddAudio::new(self.tx.clone())),
            )
            .execute(input)
            .await
            .unwrap()
        } else {
            chosen_inline::DownloadVideo::new(
                self.cfg.clone(),
                self.formatter.clone(),
                self.messenger.clone(),
                Arc::new(get_media::GetVideoByURL::new(
                    self.router.clone(),
                    self.cleaner.clone(),
                    self.tx.clone(),
                )),
                Arc::new(media::DownloadVideo::new(self.router.clone())),
                Arc::new(send_media::upload::SendVideo::new(self.messenger.clone())),
                Arc::new(send_media::id::EditVideo::new(self.messenger.clone())),
                Arc::new(downloaded_media::AddVideo::new(self.tx.clone())),
            )
            .execute(input)
            .await
            .unwrap()
        }
    }

    async fn quiet(&self, audio: bool) -> Option<SentMessage> {
        self.state.post_metadata_active.store(false, Relaxed);
        let url = Url::parse("https://example.test/playlist").unwrap();
        let language = Language::default();
        let ctx = FulfillCtx {
            chat_id: 1,
            message_id: 1,
            url: &url,
            link_is_visible: false,
            sections: None,
            audio_language: &language,
            overwrite_cache: false,
        };
        if audio {
            AudioFulfiller::new(
                self.cfg.clone(),
                self.formatter.clone(),
                Arc::new(media::DownloadAudio::new(self.router.clone())),
                Arc::new(send_media::upload::SendAudio::new(self.messenger.clone())),
                Arc::new(send_media::id::SendAudio::new(self.messenger.clone())),
                Arc::new(send_media::id::SendAudioPlaylist::new(self.messenger.clone())),
                Arc::new(downloaded_media::AddAudio::new(self.tx.clone())),
            )
            .fulfill(playlist(2), &ctx)
            .await
        } else {
            video::DownloadQuiet::new(
                self.cfg.clone(),
                self.formatter.clone(),
                Arc::new(get_media::GetVideoByURL::new(
                    self.router.clone(),
                    self.cleaner.clone(),
                    self.tx.clone(),
                )),
                Arc::new(media::DownloadVideo::new(self.router.clone())),
                Arc::new(send_media::upload::SendVideo::new(self.messenger.clone())),
                Arc::new(send_media::id::SendVideo::new(self.messenger.clone())),
                Arc::new(send_media::id::SendVideoPlaylist::new(self.messenger.clone())),
                Arc::new(downloaded_media::AddVideo::new(self.tx.clone())),
            )
            .fulfill(playlist(2), &ctx)
            .await
        }
    }
}

fn playlist(count: i16) -> GetMediaByURLKind {
    GetMediaByURLKind::Playlist {
        cached: vec![],
        uncached: (0..count)
            .map(|index| {
                (
                    Media {
                        id: index.to_string(),
                        display_id: None,
                        webpage_url: Url::parse(&format!("https://example.test/{index}")).unwrap(),
                        direct_url: Some(Url::parse(&format!("https://example.test/{index}.jpg")).unwrap()),
                        title: None,
                        language: None,
                        uploader: None,
                        duration: None,
                        playlist_index: index,
                        thumbnail: None,
                        thumbnails: vec![],
                    },
                    vec![(
                        MediaFormat {
                            format_id: index.to_string(),
                            format_note: None,
                            ext: "mp4".into(),
                            width: None,
                            height: None,
                            aspect_ratio: None,
                            filesize_approx: None,
                        },
                        "{}".into(),
                    )],
                )
            })
            .collect(),
    }
}

#[tokio::test]
async fn single_media_replaces_progress_while_albums_send_new_messages() {
    for (media_type, kind) in [
        (MediaType::Video, "video"),
        (MediaType::Audio, "audio"),
        (MediaType::Photo, "photo"),
    ] {
        let fixture = Fixture::new(State::default()).await;
        let cached = |count| GetMediaByURLKind::Playlist {
            cached: (0..count)
                .map(|index| MediaInPlaylist {
                    file_id: "synthetic-file-id".into(),
                    playlist_index: index,
                    webpage_url: None,
                })
                .collect(),
            uncached: vec![],
        };
        for (message_id, metadata) in [
            (1, GetMediaByURLKind::SingleCached("synthetic-file-id".into())),
            (1, cached(1)),
            (1, playlist(1)),
            (42, cached(2)),
            (42, playlist(2)),
        ] {
            fixture.state.events.lock().unwrap().clear();
            fixture.state.media_edits.lock().unwrap().clear();
            let message = tokio::time::timeout(Duration::from_secs(5), fixture.command_media(media_type, metadata, None))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(message.message_id, message_id);
            let events = fixture.state.events.lock().unwrap();
            let is_album = message_id == 42;
            assert_eq!(events.iter().filter(|event| *event == "delete").count(), usize::from(is_album));
            assert_eq!(
                events.iter().filter(|event| *event == "final command send").count(),
                usize::from(is_album)
            );
            assert_eq!(
                events.iter().filter(|event| *event == "final command edit").count(),
                usize::from(!is_album)
            );
            let edits = fixture.state.media_edits.lock().unwrap();
            if is_album {
                assert!(edits.is_empty());
            } else {
                assert_eq!(*edits, [format!("{kind} chat:1:1 id=synthetic-file-id caption=None")]);
            }
        }
    }
}

#[tokio::test]
async fn failed_media_edit_keeps_progress_and_reports_the_error() {
    for media_type in [MediaType::Video, MediaType::Audio, MediaType::Photo] {
        let fixture = Fixture::new(State {
            reject_media_edit: true,
            ..Default::default()
        })
        .await;
        for metadata in [GetMediaByURLKind::SingleCached("synthetic-file-id".into()), playlist(1)] {
            assert!(fixture.command_media(media_type, metadata, None).await.is_none());
            assert!(fixture
                .state
                .edits
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .contains("Synthetic media edit failure"));
        }
        let events = fixture.state.events.lock().unwrap();
        assert_eq!(events.iter().filter(|event| *event == "media edit rejected").count(), 2);
        assert!(!events.iter().any(|event| event == "delete" || event == "final command send"));
    }
}

#[tokio::test]
async fn audio_replaces_supplied_progress_and_keeps_recognition_text() {
    let fixture = Fixture::new(State::default()).await;
    let text = "<b>Synthetic artist — Synthetic song</b>";
    for metadata in [GetMediaByURLKind::SingleCached("synthetic-file-id".into()), playlist(1)] {
        let message = fixture.command_media(MediaType::Audio, metadata, Some((17, text))).await.unwrap();
        assert_eq!(message.message_id, 17);
        assert_eq!(
            fixture.state.media_edits.lock().unwrap().last().unwrap(),
            &format!("audio chat:1:17 id=synthetic-file-id caption={:?}", Some(text))
        );
    }
    assert!(!fixture
        .state
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|event| event == "delete" || event == "final command send"));
}

#[tokio::test]
async fn playlist_waits_for_upload_before_starting_next_media() {
    for audio in [false, true] {
        let fixture = Fixture::new(State::default()).await;
        let message = tokio::time::timeout(Duration::from_secs(10), fixture.command(audio, 2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message.message_id, 42);
        let events = fixture.state.events.lock().unwrap();
        let execution = events
            .iter()
            .filter(|event| event.starts_with("download ") || event.starts_with("upload "))
            .collect::<Vec<_>>();
        assert_eq!(
            execution,
            [
                "download https://example.test/playlist",
                "upload started",
                "upload finished",
                "download https://example.test/playlist",
                "upload started",
                "upload finished",
            ]
        );
    }
}

#[tokio::test]
async fn fatal_download_error_is_rendered_once_and_not_deleted() {
    for audio in [false, true] {
        for error in [
            Status::internal("Synthetic transport failure"),
            Status::deadline_exceeded("Synthetic hard timeout"),
        ] {
            let fixture = Fixture::new(State {
                replies: Mutex::new(VecDeque::from([Some(error)])),
                ..Default::default()
            })
            .await;
            let outcome = ExecutionOutcome::default();
            assert!(
                tokio::time::timeout(Duration::from_secs(10), outcome.scope(fixture.command(audio, 2)))
                    .await
                    .unwrap()
                    .is_none()
            );
            let events = fixture.state.events.lock().unwrap();
            assert_eq!(events.iter().filter(|event| event.starts_with("download ")).count(), 1);
            assert!(!events.iter().any(|event| event == "delete"));
            let edits = fixture.state.edits.lock().unwrap();
            let error_text = DownloadErrorKind::ExecutionUncertain.to_string();
            assert_eq!(edits.iter().map(|text| text.matches(&error_text).count()).sum::<usize>(), 1);
            assert!(edits.last().unwrap().contains(&error_text));
        }
    }
}

#[tokio::test]
async fn inline_ready_upload_cancels_blocked_optional_edit_before_final_result() {
    for audio in [false, true] {
        let fixture = Fixture::new(State {
            block_optional_edits: true,
            ..Default::default()
        })
        .await;
        assert!(tokio::time::timeout(Duration::from_secs(5), fixture.inline(audio)).await.unwrap());
        assert_eq!(
            *fixture.state.events.lock().unwrap(),
            [
                "download https://example.test/playlist",
                "optional edit cancelled",
                "upload started",
                "upload finished",
                "optional edit cancelled",
                "final inline edit",
            ]
        );
    }
}

#[tokio::test]
async fn sending_status_is_visible_during_upload_instead_of_blocking_it() {
    for audio in [false, true] {
        let fixture = Fixture::new(State::default()).await;
        assert!(tokio::time::timeout(Duration::from_secs(5), fixture.inline(audio)).await.unwrap());
        assert_eq!(
            *fixture.state.events.lock().unwrap(),
            [
                "download https://example.test/playlist",
                "upload started",
                "sending",
                "upload finished",
                "final inline edit",
            ]
        );
        let fixture = Fixture::new(State::default()).await;
        tokio::time::timeout(Duration::from_secs(5), fixture.command(audio, 1))
            .await
            .unwrap();
        let events = fixture.state.events.lock().unwrap();
        let started = events.iter().position(|event| event == "upload started").unwrap();
        let sending = events.iter().position(|event| event == "sending").unwrap();
        let finished = events.iter().position(|event| event == "upload finished").unwrap();
        assert!(started < sending && sending < finished);
    }
}

#[tokio::test]
async fn rejected_upload_is_not_retried_and_tells_user_to_resend() {
    for audio in [false, true] {
        for inline in [false, true] {
            let fixture = Fixture::new(State {
                reject_upload: true,
                ..Default::default()
            })
            .await;
            tokio::time::timeout(Duration::from_secs(5), async {
                if inline {
                    assert!(!fixture.inline(audio).await);
                } else {
                    assert!(fixture.command(audio, 1).await.is_none());
                }
            })
            .await
            .unwrap();
            let events = fixture.state.events.lock().unwrap();
            assert_eq!(events.iter().filter(|event| event.starts_with("download ")).count(), 1);
            assert_eq!(events.iter().filter(|event| *event == "upload started").count(), 1);
            assert!(!events.iter().any(|event| event == "delete" || event == "final inline edit"));
            let edits = fixture.state.edits.lock().unwrap();
            let final_error = edits.last().unwrap();
            assert!(final_error.contains("retry after 2"));
            assert!(final_error.contains("resend the link manually"));
        }
    }
}

#[tokio::test]
async fn known_failed_playlist_item_does_not_stop_next_item() {
    for audio in [false, true] {
        let mut error = Status::internal("Synthetic known failure");
        error.metadata_mut().insert("x-download-terminal", "true".parse().unwrap());
        let fixture = Fixture::new(State {
            replies: Mutex::new(VecDeque::from([Some(error), None])),
            ..Default::default()
        })
        .await;
        let message = tokio::time::timeout(Duration::from_secs(10), fixture.command(audio, 2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message.message_id, 1);
        let events = fixture.state.events.lock().unwrap();
        assert_eq!(events.iter().filter(|event| event.starts_with("download ")).count(), 2);
        assert_eq!(events.iter().filter(|event| *event == "upload finished").count(), 1);
        assert!(!events.iter().any(|event| event == "delete"));
        let edits = fixture.state.edits.lock().unwrap();
        assert_eq!(edits.iter().filter(|text| text.contains("Synthetic known failure")).count(), 1);
    }
}

#[tokio::test]
async fn quiet_playlists_also_finish_each_upload_before_next_download() {
    for audio in [false, true] {
        let fixture = Fixture::new(State::default()).await;
        let message = tokio::time::timeout(Duration::from_secs(10), fixture.quiet(audio))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message.message_id, 42);
        assert_eq!(
            *fixture.state.events.lock().unwrap(),
            [
                "download https://example.test/playlist",
                "upload started",
                "upload finished",
                "download https://example.test/playlist",
                "upload started",
                "upload finished",
                "final command send",
            ]
        );
        assert!(fixture.state.edits.lock().unwrap().is_empty());
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn post_text_enriches_delivered_media_without_repeating_downloads() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let short = "Synthetic <caption> & text";
        let footer = "Synthetic Author | <a href=\"https://example.test/playlist\">Link</a>";
        let short_rendered = format!("Synthetic &lt;caption&gt; &amp; text\n\n{footer}");
        let long = "Long caption 😀<&> ".repeat(110);
        let very_long = "Long post 😀<&> ".repeat(500);
        let assert_truncated = |event: &str, prefix: &str, limit: usize| {
            let html = event.strip_prefix(prefix).unwrap();
            assert!(html.contains("<blockquote expandable>"));
            assert!(html.contains("&lt;") && html.contains("&amp;"));
            let (body, source) = html.rsplit_once("\n\n").unwrap();
            assert!(body.ends_with("…</blockquote>"));
            assert_eq!(source, footer);
            let visible = regex::Regex::new("<[^>]*>")
                .unwrap()
                .replace_all(html, "")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&");
            assert!(visible.encode_utf16().count() <= limit);
            assert!(visible.ends_with("Synthetic Author | Link"));
            assert!(visible.contains('😀'));
        };
        let response = |text: &str| {
            Ok(MediaInfoResponse {
                posts: vec![PostEntry {
                    text: text.into(),
                    author: Some("Synthetic Author".into()),
                    webpage_url: "https://example.test/playlist".into(),
                    ..Default::default()
                }],
                ..Default::default()
            })
        };
        let mut hidden = response(short).unwrap();
        hidden.posts.push(PostEntry {
            text: "Synthetic <context> & text".into(),
            author: Some("Synthetic Context Author".into()),
            author_handle: Some("synthetic_context".into()),
            webpage_url: "https://example.test/context".into(),
        });
        let fixture = Fixture::new(State {
            post_replies: Mutex::new(VecDeque::from([
                response(short),
                response(short),
                Err(Status::not_found("Synthetic metadata failure")),
                response(&long),
                response(short),
                response(&long),
                response(short),
                response(&very_long),
                response(short),
                response(&very_long),
                Ok(hidden),
                response(short),
                response(short),
                response(short),
            ])),
            ..Default::default()
        })
        .await;
        let display = post::Display::new(fixture.router.clone(), fixture.messenger.clone(), fixture.formatter.clone());
        let mut job = DownloadJob::new(
            MediaType::Video,
            Some(Url::parse("https://example.test/playlist").unwrap()),
            Params::default(),
            ChatConfig::new(1, false, "en".into()),
            true,
            JobTarget::Command { chat_id: 1, message_id: 9 },
        )
        .with_auto(false);

        let message = fixture.command(false, 1).await.unwrap();
        assert_eq!(message.message_id, 1);
        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: Some(&message),
                has_media: true
            })
            .await
            .unwrap());
        assert_eq!(
            fixture.state.events.lock().unwrap().last().unwrap(),
            &format!("post edit chat:1:1 caption=true: {short_rendered}")
        );

        let edits = fixture.state.edits.lock().unwrap().len();
        *fixture.state.post_edit_error.lock().unwrap() = Some(MessengerError::new("Synthetic caption edit failure"));
        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: Some(&message),
                has_media: true
            })
            .await
            .unwrap());
        assert_eq!(fixture.state.edits.lock().unwrap().len(), edits);
        assert_eq!(fixture.state.events.lock().unwrap().last().unwrap(), "post edit rejected");

        assert!(!display
            .execute(post::DisplayInput {
                job: &job,
                message: Some(&message),
                has_media: true
            })
            .await
            .unwrap());
        assert_eq!(fixture.state.edits.lock().unwrap().len(), edits);
        assert_eq!(fixture.state.events.lock().unwrap().last().unwrap(), "post metadata");

        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: Some(&message),
                has_media: true
            })
            .await
            .unwrap());
        assert_truncated(
            fixture.state.events.lock().unwrap().last().unwrap(),
            "post edit chat:1:1 caption=true: ",
            1_024,
        );

        assert!(fixture.inline(false).await);
        job.target = JobTarget::Inline {
            inline_message_id: "synthetic-inline".into(),
            result_id: "synthetic-result".into(),
        };
        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: None,
                has_media: true
            })
            .await
            .unwrap());
        assert_eq!(
            fixture.state.events.lock().unwrap().last().unwrap(),
            &format!("post edit inline:synthetic-inline caption=true: {short_rendered}")
        );
        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: None,
                has_media: true
            })
            .await
            .unwrap());
        assert_truncated(
            fixture.state.events.lock().unwrap().last().unwrap(),
            "post edit inline:synthetic-inline caption=true: ",
            1_024,
        );

        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: None,
                has_media: false
            })
            .await
            .unwrap());
        assert_eq!(
            fixture.state.events.lock().unwrap().last().unwrap(),
            &format!("post edit inline:synthetic-inline caption=false: {short_rendered}")
        );
        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: None,
                has_media: false
            })
            .await
            .unwrap());
        assert_truncated(
            fixture.state.events.lock().unwrap().last().unwrap(),
            "post edit inline:synthetic-inline caption=false: ",
            4_096,
        );

        job.target = JobTarget::Command { chat_id: 1, message_id: 9 };
        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: None,
                has_media: false
            })
            .await
            .unwrap());
        assert_eq!(
            fixture.state.events.lock().unwrap().last().unwrap(),
            &format!("post text 1 reply=Some(9): {short_rendered}")
        );
        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: None,
                has_media: false
            })
            .await
            .unwrap());
        assert_truncated(
            fixture.state.events.lock().unwrap().last().unwrap(),
            "post text 1 reply=Some(9): ",
            4_096,
        );

        job.link_is_visible = false;
        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: Some(&message),
                has_media: true
            })
            .await
            .unwrap());
        let hidden = fixture.state.events.lock().unwrap().last().unwrap().clone();
        let hidden = hidden.strip_prefix("post edit chat:1:1 caption=true: ").unwrap();
        assert!(hidden.contains("Synthetic &lt;caption&gt; &amp; text"));
        assert!(hidden.contains("<b>Synthetic Context Author (@synthetic_context)</b>\nSynthetic &lt;context&gt; &amp; text"));
        assert!(hidden.ends_with("\n\nSynthetic Author"));
        assert!(!hidden.contains("href=") && !hidden.contains("https://example.test/"));

        *fixture.state.post_send_error.lock().unwrap() = Some(MessengerError::new("Synthetic <delivery> failure & text token=test"));
        let error = display
            .execute(post::DisplayInput {
                job: &job,
                message: None,
                has_media: false,
            })
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Messenger error: Synthetic <delivery> failure & text token=...");
        let command_error = fixture.state.events.lock().unwrap().last().unwrap().clone();
        assert!(command_error.starts_with("post text 1 reply=Some(9): Failed to send message\n"));
        assert!(command_error
            .contains("<blockquote expandable>Messenger error: Synthetic &lt;delivery&gt; failure &amp; text token=...</blockquote>"));

        job.target = JobTarget::Inline {
            inline_message_id: "synthetic-inline".into(),
            result_id: "synthetic-result".into(),
        };
        *fixture.state.post_edit_error.lock().unwrap() = Some(MessengerError::new("Synthetic <edit> failure & text"));
        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: None,
                has_media: false
            })
            .await
            .is_err());
        let inline_error = fixture.state.events.lock().unwrap().last().unwrap().clone();
        assert!(inline_error.starts_with("post edit inline:synthetic-inline caption=false: Failed to send message\n"));
        assert!(inline_error.contains("<blockquote expandable>Messenger error: Synthetic &lt;edit&gt; failure &amp; text</blockquote>"));

        job.target = JobTarget::Command { chat_id: 1, message_id: 9 };
        job.quiet = true;
        *fixture.state.post_send_error.lock().unwrap() = Some(MessengerError::new("Synthetic quiet delivery failure"));
        assert!(display
            .execute(post::DisplayInput {
                job: &job,
                message: None,
                has_media: false
            })
            .await
            .is_err());
        assert_eq!(fixture.state.events.lock().unwrap().last().unwrap(), "post send rejected");

        let events = fixture.state.events.lock().unwrap();
        assert_eq!(events.iter().filter(|event| event.starts_with("post text ")).count(), 3);
        assert!(!events
            .iter()
            .any(|event| event.starts_with("post text 1 reply=Some(1):") || event.starts_with("post text 1 reply=None:")));
        assert_eq!(events.iter().filter(|event| *event == "post metadata").count(), 14);
        assert_eq!(events.iter().filter(|event| event.starts_with("download ")).count(), 2);
        assert_eq!(events.iter().filter(|event| *event == "upload finished").count(), 2);
        assert_eq!(events.iter().filter(|event| *event == "final command edit").count(), 1);
        assert_eq!(events.iter().filter(|event| *event == "final inline edit").count(), 1);
        assert!(fixture.state.post_replies.lock().unwrap().is_empty());
    })
    .await
    .unwrap();
}
