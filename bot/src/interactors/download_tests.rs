use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use downloader_client::{
    outcome::ExecutionOutcome, DownloadErrorKind, DownloaderClusterConfig, DownloaderServiceTarget, DownloaderTlsConfig, NodeRouter,
};
use futures_util::{stream, Stream, StreamExt as _};
use proto::downloader::{
    download_chunk::Payload,
    downloader_server::{Downloader, DownloaderServer},
    node_capabilities_server::{NodeCapabilities, NodeCapabilitiesServer},
    DownloadChunk, DownloadMeta, DownloadRequest, Empty, MediaInfoRequest, MediaInfoResponse, NodeStatus, SupportedDomainsResponse,
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
    chosen_inline, video, Interactor,
};
use crate::{
    config::Config,
    database::{SeaOrmTxManager, TxManager, TxManagerFactories},
    entities::{language::Language, ChatConfig, Media, MediaForUpload, MediaFormat, Params},
    services::{
        download::media,
        downloaded_media,
        get_media::{self, GetMediaByURLKind},
        messenger::{
            AnswerInlineErrorRequest, AnswerInlineQueryRequest, DeleteMessageRequest, EditMediaByIdRequest, EditTextRequest,
            MessengerError, MessengerPort, SendMediaByIdRequest, SendMediaGroupRequest, SendTextRequest, SentMessage, UploadAudioRequest,
            UploadPhotoRequest, UploadPhotoUrlRequest, UploadVideoRequest,
        },
        send_media,
    },
    utils::{ErrorFormatter, UrlCleaner},
};

#[derive(Default)]
struct State {
    events: Mutex<Vec<String>>,
    replies: Mutex<VecDeque<Option<Status>>>,
    edits: Mutex<Vec<String>>,
    optional_edit_started: Notify,
    block_optional_edits: bool,
    reject_upload: bool,
    reject_guest_reply: bool,
    staging_links: Mutex<Vec<bool>>,
}

impl State {
    fn event(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }
}

struct FakeNode(Arc<State>);

#[tonic::async_trait]
impl Downloader for FakeNode {
    type DownloadMediaStream = Pin<Box<dyn Stream<Item = Result<DownloadChunk, Status>> + Send>>;

    async fn get_media_info(&self, _: Request<MediaInfoRequest>) -> Result<Response<MediaInfoResponse>, Status> {
        panic!("Tests use prefetched metadata");
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
    async fn answer_guest(&self, request: crate::services::messenger::AnswerGuestRequest<'_>) -> Result<String, MessengerError> {
        self.0.event(format!("guest reply: {}", request.text));
        if self.0.reject_guest_reply {
            Err(MessengerError::new("Synthetic ambiguous reply"))
        } else {
            Ok("synthetic-guest-message".into())
        }
    }

    async fn username(&self) -> Result<String, MessengerError> {
        Ok("test_bot".into())
    }
    async fn send_text(&self, _: SendTextRequest<'_>) -> Result<SentMessage, MessengerError> {
        Ok(SentMessage { message_id: 1 })
    }
    async fn edit_text(&self, request: EditTextRequest<'_>) -> Result<(), MessengerError> {
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
    async fn delete_message(&self, _: DeleteMessageRequest) -> Result<(), MessengerError> {
        self.0.event("delete");
        Ok(())
    }
    async fn upload_video(&self, request: UploadVideoRequest<'_>) -> Result<Box<str>, MessengerError> {
        self.0.staging_links.lock().unwrap().push(request.link_is_visible);
        self.upload(request.media_for_upload).await
    }
    async fn upload_audio(&self, request: UploadAudioRequest<'_>) -> Result<Box<str>, MessengerError> {
        self.0.staging_links.lock().unwrap().push(request.link_is_visible);
        self.upload(request.media_for_upload).await
    }
    async fn send_video_by_id(&self, _: SendMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        Ok(())
    }
    async fn send_audio_by_id(&self, _: SendMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        Ok(())
    }
    async fn send_video_group(&self, _: SendMediaGroupRequest) -> Result<(), MessengerError> {
        Ok(())
    }
    async fn send_audio_group(&self, _: SendMediaGroupRequest) -> Result<(), MessengerError> {
        Ok(())
    }
    async fn edit_video_by_id(&self, _: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        self.0.event("final inline edit");
        Ok(())
    }
    async fn edit_audio_by_id(&self, _: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        self.0.event("final inline edit");
        Ok(())
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
        self.0.staging_links.lock().unwrap().push(request.link_is_visible);
        self.0.event(format!("photo upload {}", request.photo_url));
        Ok("synthetic-photo-id".into())
    }
    async fn send_photo_by_id(&self, _: SendMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn edit_photo_by_id(&self, request: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        assert!(!request.link_is_visible);
        self.0.event(format!("photo edit {}", request.remote_id));
        Ok(())
    }
    async fn send_photo_group(&self, _: SendMediaGroupRequest) -> Result<(), MessengerError> {
        unreachable!()
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
        let router = Arc::new(NodeRouter::new(
            &DownloaderClusterConfig {
                node_token: "test".into(),
                tls: DownloaderTlsConfig {
                    ca_cert: concat!(env!("CARGO_MANIFEST_DIR"), "/src/interactors/test_data/cert.pem").into(),
                    cert: concat!(env!("CARGO_MANIFEST_DIR"), "/src/interactors/test_data/cert.pem").into(),
                    key: concat!(env!("CARGO_MANIFEST_DIR"), "/src/interactors/test_data/key.pem").into(),
                },
            },
            1_000_000,
            Arc::new(DownloaderServiceTarget {
                host: "127.0.0.1".into(),
                port,
            }),
        ));
        router.refresh_status().await;
        assert_eq!(router.subscribe_capacity().borrow().available, 5);
        let cfg: Config = serde_json::from_value(serde_json::json!({
            "bot": {"token": "test", "src_url": "https://example.test"},
            "chat": {"receiver_chat_id": 1}, "logging": {"dirs": "info"},
            "database": {"host": "unused", "port": 5432, "user": "", "password": "", "database": ""},
            "redis": {"host": "unused", "port": 6379},
            "yt_dlp": {"max_file_size": 1000000}, "yt_toolkit": {"url": "https://example.test"},
            "download": {"node_token": "test", "tls": {"ca_cert_path": "", "cert_path": "", "key_path": ""}},
            "telegram_bot_api": {"url": "https://example.test"}
        }))
        .unwrap();
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

    async fn command(&self, audio: bool, count: i16) {
        let url = Url::parse("https://example.test/playlist").unwrap();
        let params = Params::default();
        let chat = ChatConfig::new(1, false, "en".into());
        let current = watch::channel(None).0;
        if audio {
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
                    prefetched: Some(playlist(count)),
                    base_text: None,
                    progress_message_id: None,
                })
                .await
                .unwrap();
        } else {
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
                    prefetched: Some(playlist(count)),
                })
                .await
                .unwrap();
        }
    }

    async fn inline(&self, audio: bool) {
        self.inline_with_guest(audio, false).await;
    }

    async fn inline_with_guest(&self, audio: bool, guest: bool) {
        let url = Url::parse("https://example.test/playlist").unwrap();
        let params = Params::default();
        let chat = ChatConfig::new(1, false, "en".into());
        let input = chosen_inline::DownloadInput {
            guest,
            params: &params,
            url: Some(&url),
            chat_cfg: &chat,
            link_is_visible: false,
            inline_message_id: "synthetic-inline",
            result_id: "synthetic-result",
            prefetched: Some(playlist(if guest { 3 } else { 1 })),
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
            .unwrap();
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
            .unwrap();
        }
    }

    async fn quiet(&self, audio: bool) {
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
            .await;
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
            .await;
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
                        direct_url: None,
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
async fn playlist_waits_for_upload_before_starting_next_media() {
    for audio in [false, true] {
        let fixture = Fixture::new(State::default()).await;
        tokio::time::timeout(Duration::from_secs(10), fixture.command(audio, 2))
            .await
            .unwrap();
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
            tokio::time::timeout(Duration::from_secs(10), outcome.scope(fixture.command(audio, 2)))
                .await
                .unwrap();
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
        tokio::time::timeout(Duration::from_secs(5), fixture.inline(audio)).await.unwrap();
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
        tokio::time::timeout(Duration::from_secs(5), fixture.inline(audio)).await.unwrap();
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
                    fixture.inline(audio).await;
                } else {
                    fixture.command(audio, 1).await;
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
        tokio::time::timeout(Duration::from_secs(10), fixture.command(audio, 2))
            .await
            .unwrap();
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
        tokio::time::timeout(Duration::from_secs(10), fixture.quiet(audio)).await.unwrap();
        assert_eq!(
            *fixture.state.events.lock().unwrap(),
            [
                "download https://example.test/playlist",
                "upload started",
                "upload finished",
                "download https://example.test/playlist",
                "upload started",
                "upload finished",
            ]
        );
        assert!(fixture.state.edits.lock().unwrap().is_empty());
    }
}

#[derive(Default)]
struct GuestQueue {
    reserved: Mutex<std::collections::HashSet<String>>,
    jobs: Mutex<Vec<crate::entities::DownloadJob>>,
    fail_reservation: bool,
    fail_enqueue: bool,
}

impl crate::services::queue::GuestQueuePort for GuestQueue {
    async fn reserve_guest_query(&self, id: &str) -> Result<bool, crate::services::queue::QueueError> {
        if self.fail_reservation {
            return Err(redis::RedisError::from((redis::ErrorKind::IoError, "Synthetic reservation failure")).into());
        }
        Ok(self.reserved.lock().unwrap().insert(id.into()))
    }

    async fn enqueue_guest(&self, job: &crate::entities::DownloadJob) -> Result<(), crate::services::queue::QueueError> {
        // Model an ambiguous write: Redis stored the job but its response was lost.
        self.jobs.lock().unwrap().push(job.clone());
        if self.fail_enqueue {
            return Err(redis::RedisError::from((redis::ErrorKind::IoError, "Synthetic enqueue failure")).into());
        }
        Ok(())
    }
}

fn guest_interactor(state: Arc<State>, queue: Arc<GuestQueue>) -> super::guest::EnqueueGuestDownload<RecordingMessenger, GuestQueue> {
    let cfg: Config = serde_json::from_value(serde_json::json!({
        "bot": {"token": "test", "src_url": "https://example.test"},
        "chat": {"receiver_chat_id": 1}, "logging": {"dirs": "info"},
        "database": {"host": "unused", "port": 5432, "user": "", "password": "", "database": ""},
        "redis": {"host": "unused", "port": 6379},
        "yt_dlp": {"max_file_size": 1_000_000}, "yt_toolkit": {"url": "https://example.test"},
        "download": {"node_token": "test", "tls": {"ca_cert_path": "", "cert_path": "", "key_path": ""}},
        "telegram_bot_api": {"url": "https://example.test"},
        "tracking_params": {"params": ["tracking"]},
        "blacklisted": {"domains": ["blocked.example.test"]}
    }))
    .unwrap();
    let cleaner = Arc::new(UrlCleaner::from_embedded_rules(&cfg.tracking_params).unwrap());
    super::guest::EnqueueGuestDownload::new(Arc::new(RecordingMessenger(state)), queue, cleaner, Arc::new(cfg))
}

fn guest_input(url: &str) -> super::guest::GuestInput<'static> {
    super::guest::GuestInput {
        query_id: "synthetic-guest-query",
        url: Some(Url::parse(url).unwrap()),
        locale: crate::locale::Locale::Uk,
        media_type: None,
    }
}

#[tokio::test]
async fn guest_enqueues_once_without_chat_or_query_context() {
    let state = Arc::new(State::default());
    let queue = Arc::new(GuestQueue::default());
    let interactor = guest_interactor(state.clone(), queue.clone());
    for _ in 0..2 {
        (&interactor)
            .execute(guest_input("https://example.test/media?tracking=synthetic"))
            .await
            .unwrap();
    }
    let jobs = queue.jobs.lock().unwrap();
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert!(job.auto && job.guest);
    assert_eq!(job.chat_cfg.tg_id, 0);
    assert_eq!(job.chat_cfg.language, "uk");
    assert!(!job.link_is_visible && !job.chat_cfg.cmd_random_enabled);
    assert_eq!(job.url.as_ref().unwrap().as_str(), "https://example.test/media");
    assert!(job.params.0.is_empty());
    let payload = serde_json::to_string(job).unwrap();
    assert!(!payload.contains("synthetic-guest-query"));
    let decoded: crate::entities::DownloadJob = serde_json::from_str(&payload).unwrap();
    assert!(decoded.guest);
    assert!(
        matches!(&decoded.target, crate::entities::JobTarget::Inline { inline_message_id, .. } if inline_message_id == "synthetic-guest-message")
    );
    assert_eq!(state.events.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn guest_explicit_media_type_disables_auto() {
    let state = Arc::new(State::default());
    let queue = Arc::new(GuestQueue::default());
    let interactor = guest_interactor(state, queue.clone());
    let mut input = guest_input("https://example.test/media");
    input.media_type = Some(crate::value_objects::MediaType::Audio);
    (&interactor).execute(input).await.unwrap();
    let jobs = queue.jobs.lock().unwrap();
    assert!(!jobs[0].auto);
    assert!(matches!(jobs[0].media_type, crate::value_objects::MediaType::Audio));
}

#[tokio::test]
async fn guest_invalid_or_blocked_link_replies_without_download() {
    for url in [
        "file:///tmp/synthetic",
        "https://user:secret@example.test/media",
        "https://blocked.example.test/media",
        "http://127.0.0.1/media",
        "http://192.168.1.10/media",
        "http://10/media",
        "http://2130706433/media",
        "http://0x7f.1/media",
        "http://[::1]/media",
    ] {
        let state = Arc::new(State::default());
        let queue = Arc::new(GuestQueue::default());
        let interactor = guest_interactor(state.clone(), queue.clone());
        (&interactor).execute(guest_input(url)).await.unwrap();
        assert!(queue.jobs.lock().unwrap().is_empty());
        let events = state.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("Згадайте"));
        assert!(!events[0].contains(url));
    }
}

#[tokio::test]
async fn guest_ambiguous_reply_is_not_replayed_or_downloaded() {
    let state = Arc::new(State {
        reject_guest_reply: true,
        ..Default::default()
    });
    let queue = Arc::new(GuestQueue::default());
    let interactor = guest_interactor(state.clone(), queue.clone());
    for _ in 0..2 {
        (&interactor).execute(guest_input("https://example.test/media")).await.unwrap();
    }
    assert!(queue.jobs.lock().unwrap().is_empty());
    assert_eq!(state.events.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn guest_reservation_failure_sends_no_untracked_reply() {
    let state = Arc::new(State::default());
    let queue = Arc::new(GuestQueue {
        fail_reservation: true,
        ..Default::default()
    });
    let interactor = guest_interactor(state.clone(), queue.clone());
    (&interactor).execute(guest_input("https://example.test/media")).await.unwrap();
    assert!(state.events.lock().unwrap().is_empty());
    assert!(queue.jobs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn guest_ambiguous_enqueue_edits_placeholder_without_retry() {
    let state = Arc::new(State::default());
    let queue = Arc::new(GuestQueue {
        fail_enqueue: true,
        ..Default::default()
    });
    let interactor = guest_interactor(state.clone(), queue.clone());
    for _ in 0..2 {
        (&interactor).execute(guest_input("https://example.test/media")).await.unwrap();
    }
    assert_eq!(queue.jobs.lock().unwrap().len(), 1);
    assert_eq!(state.events.lock().unwrap().len(), 1);
    let edits = state.edits.lock().unwrap();
    assert_eq!(edits.len(), 1);
    assert!(edits[0].contains("підтвердити"));
    assert!(!edits[0].contains("Synthetic"));
}

#[tokio::test]
async fn guest_streams_media_and_hides_staging_link() {
    for audio in [false, true] {
        let fixture = Fixture::new(State::default()).await;
        fixture.inline_with_guest(audio, true).await;
        let events = fixture.state.events.lock().unwrap();
        assert_eq!(*fixture.state.staging_links.lock().unwrap(), [false]);
        assert!(events.iter().any(|event| event == "final inline edit"));
        assert_eq!(events.iter().filter(|event| event.starts_with("download ")).count(), 1);
    }
}

#[tokio::test]
async fn guest_errors_do_not_publish_downloader_diagnostics() {
    for audio in [false, true] {
        let mut error = Status::internal("Synthetic private source URL https://example.test/private?secret=synthetic");
        error.metadata_mut().insert("x-download-terminal", "true".parse().unwrap());
        let fixture = Fixture::new(State {
            replies: Mutex::new(VecDeque::from([Some(error)])),
            ..Default::default()
        })
        .await;
        fixture.inline_with_guest(audio, true).await;
        let edits = fixture.state.edits.lock().unwrap();
        assert!(!edits.is_empty());
        assert!(edits.iter().all(|text| !text.contains("Synthetic") && !text.contains("secret=")));
    }
}

#[tokio::test]
async fn guest_photo_selects_first_item_across_cache_hits_and_misses() {
    // Metadata can contain a full album. Cache state must never change which photo wins.
    for cached_index in [None, Some(0), Some(1)] {
        let fixture = Fixture::new(State::default()).await;
        let GetMediaByURLKind::Playlist { mut uncached, .. } = playlist(3) else {
            unreachable!()
        };
        for (media, _) in &mut uncached {
            media.direct_url = Some(media.webpage_url.clone());
        }
        let cached = cached_index.map_or_else(Vec::new, |index| {
            let (media, _) = uncached.remove(index);
            vec![crate::entities::MediaInPlaylist {
                file_id: format!("cached-photo-{index}"),
                playlist_index: media.playlist_index,
                webpage_url: Some(media.webpage_url),
            }]
        });
        uncached.reverse(); // Selection must use playlist indices, not vector order.
        let url = Url::parse("https://example.test/album").unwrap();
        let params = Params::default();
        let chat = ChatConfig::new(0, false, "en".into());
        chosen_inline::DownloadPhoto::new(
            fixture.cfg.clone(),
            fixture.formatter.clone(),
            fixture.messenger.clone(),
            Arc::new(get_media::GetPhotoByURL::new(
                fixture.router.clone(),
                fixture.cleaner.clone(),
                fixture.tx.clone(),
            )),
            Arc::new(send_media::upload::SendPhotoUrl::new(fixture.messenger.clone())),
            Arc::new(send_media::id::EditPhoto::new(fixture.messenger.clone())),
            Arc::new(downloaded_media::AddPhoto::new(fixture.tx.clone())),
        )
        .execute(chosen_inline::DownloadInput {
            guest: true,
            params: &params,
            url: Some(&url),
            chat_cfg: &chat,
            link_is_visible: false,
            inline_message_id: "synthetic-inline",
            result_id: "guest",
            prefetched: Some(GetMediaByURLKind::Playlist { cached, uncached }),
        })
        .await
        .unwrap();
        let events = fixture.state.events.lock().unwrap();
        if cached_index == Some(0) {
            assert_eq!(*events, ["photo edit cached-photo-0"]);
            assert!(fixture.state.staging_links.lock().unwrap().is_empty());
        } else {
            assert_eq!(*events, ["photo upload https://example.test/0", "photo edit synthetic-photo-id"]);
            assert_eq!(*fixture.state.staging_links.lock().unwrap(), [false]);
        }
    }
}
