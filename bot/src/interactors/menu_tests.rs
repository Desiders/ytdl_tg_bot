use std::sync::{Arc, Mutex};

use downloader_client::{DownloaderClusterConfig, DownloaderServiceTarget, DownloaderTlsConfig, NodeRouter};
use migration::{Migrator, MigratorTrait as _};
use rust_i18n::t;
use sea_orm::{ConnectOptions, Database};
use telers::fsm::{MemoryStorage, StorageKey};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner as _, ContainerAsync, ImageExt as _},
};

use super::{
    config::MAX_EXCLUDE_DOMAINS,
    menu::{
        AddMenuDomain, AddMenuDomainInput, Fsm, MenuTarget, OpenMenu, OpenMenuInput, RemoveMenuDomain, RemoveMenuDomainInput,
        SetMenuLanguage, SetMenuLanguageInput, SetMenuLinkVisibility, SetMenuLinkVisibilityInput,
    },
    stats::Stats,
    Interactor,
};
use crate::{
    config::Config,
    database::{SeaOrmTxManager, TxManager, TxManagerFactories},
    entities::{Chat, ChatConfig, ChatConfigExcludeDomain, ChatConfigExcludeDomains},
    locale::Locale,
    services::{
        chat, downloaded_media,
        messenger::{
            AnswerCallbackRequest, AnswerInlineErrorRequest, AnswerInlineQueryRequest, ButtonAction, DeleteMessageRequest,
            EditMediaByIdRequest, EditMenuRequest, EditTextRequest, Keyboard, MessengerError, MessengerPort, SendMediaByIdRequest,
            SendMediaGroupRequest, SendMenuRequest, SendTextRequest, SentMessage, UploadAudioRequest, UploadPhotoRequest,
            UploadPhotoUrlRequest, UploadVideoRequest,
        },
        node_router, queue,
    },
    utils::ErrorFormatter,
    value_objects::{
        menu::{DomainKey, MenuState, Screen},
        ChatType,
    },
};

const CHAT_ID: i64 = 1;
const CALLBACK_ID: &str = "synthetic-callback";
const EDIT: MenuTarget<'static> = MenuTarget::Edit {
    chat_id: CHAT_ID,
    message_id: 2,
    callback_id: CALLBACK_ID,
};

struct RecordingMessenger(Arc<Mutex<Vec<String>>>);

impl RecordingMessenger {
    fn record(&self, event: String) {
        self.0.lock().unwrap().push(event);
    }
}

impl MessengerPort for RecordingMessenger {
    async fn username(&self) -> Result<String, MessengerError> {
        Ok("test_bot".into())
    }
    async fn send_menu(&self, request: SendMenuRequest<'_>) -> Result<(), MessengerError> {
        self.record(format!("send {}", menu(request.text, request.keyboard)));
        Ok(())
    }
    async fn edit_menu(&self, request: EditMenuRequest<'_>) -> Result<(), MessengerError> {
        self.record(format!("edit {}", menu(request.text, request.keyboard)));
        Ok(())
    }
    async fn answer_callback(&self, request: AnswerCallbackRequest<'_>) -> Result<(), MessengerError> {
        assert_eq!(request.callback_id, CALLBACK_ID);
        self.record(format!("answer {:?}", request.text));
        Ok(())
    }
    async fn send_text(&self, _: SendTextRequest<'_>) -> Result<SentMessage, MessengerError> {
        unreachable!()
    }
    async fn edit_text(&self, _: EditTextRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn delete_message(&self, _: DeleteMessageRequest) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn answer_inline_error(&self, _: AnswerInlineErrorRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn answer_inline_query(&self, _: AnswerInlineQueryRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn upload_video(&self, _: UploadVideoRequest<'_>) -> Result<Box<str>, MessengerError> {
        unreachable!()
    }
    async fn upload_audio(&self, _: UploadAudioRequest<'_>) -> Result<Box<str>, MessengerError> {
        unreachable!()
    }
    async fn upload_photo(&self, _: UploadPhotoRequest<'_>) -> Result<Box<str>, MessengerError> {
        unreachable!()
    }
    async fn upload_photo_url(&self, _: UploadPhotoUrlRequest<'_>) -> Result<Box<str>, MessengerError> {
        unreachable!()
    }
    async fn send_video_by_id(&self, _: SendMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn send_audio_by_id(&self, _: SendMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn send_photo_by_id(&self, _: SendMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn edit_video_by_id(&self, _: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn edit_audio_by_id(&self, _: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn edit_photo_by_id(&self, _: EditMediaByIdRequest<'_>) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn send_video_group(&self, _: SendMediaGroupRequest) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn send_audio_group(&self, _: SendMediaGroupRequest) -> Result<(), MessengerError> {
        unreachable!()
    }
    async fn send_photo_group(&self, _: SendMediaGroupRequest) -> Result<(), MessengerError> {
        unreachable!()
    }
}

// The keyboard is recorded too, since toggling link visibility changes only a button.
fn menu(text: &str, keyboard: &Keyboard) -> String {
    let buttons: Vec<String> = keyboard
        .rows
        .iter()
        .flatten()
        .map(|button| match &button.action {
            ButtonAction::Callback(data) => format!("[{} {data}]", button.text),
            ButtonAction::Url(url) => format!("[{} {url}]", button.text),
        })
        .collect();
    format!("{text}\n{}", buttons.join(" "))
}

struct Fixture {
    events: Arc<Mutex<Vec<String>>>,
    tx_manager: Arc<Box<dyn TxManager>>,
    formatter: Arc<ErrorFormatter>,
    open_menu: Arc<OpenMenu<RecordingMessenger>>,
    fsm: Fsm,
    _postgres: ContainerAsync<Postgres>,
    _redis: queue::test_support::Fixture,
}

struct Saved {
    config: ChatConfig,
    exclude_domains: ChatConfigExcludeDomains,
}

impl Fixture {
    async fn new(domains: &[String]) -> Self {
        let postgres = Postgres::default().with_tag("18-alpine").start().await.unwrap();
        let port = postgres.get_host_port_ipv4(5432).await.unwrap();
        let conn = Database::connect(ConnectOptions::new(format!(
            "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
        )))
        .await
        .unwrap();
        Migrator::up(&conn, None).await.unwrap();

        let tx_manager: Arc<Box<dyn TxManager>> = Arc::new(Box::new(SeaOrmTxManager::new(
            Arc::new(conn),
            Arc::new(TxManagerFactories::default()),
        )));

        chat::SaveChat::new(tx_manager.clone())
            .execute(chat::SaveChatInput {
                chat: Chat::new(CHAT_ID, None, ChatType::Private),
                chat_config: ChatConfig::new(CHAT_ID, false, "en".into()),
            })
            .await
            .unwrap();
        for domain in domains {
            chat::AddExcludeDomain::new(tx_manager.clone())
                .execute(chat::ExcludeDomainInput {
                    dto: ChatConfigExcludeDomain::new(CHAT_ID, domain.clone()),
                })
                .await
                .unwrap();
        }

        let redis = queue::test_support::Fixture::new().await;
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
                port: 1,
            }),
        ));
        let config: Config = serde_json::from_value(serde_json::json!({
            "bot": {"token": "test", "src_url": "https://example.test"},
            "chat": {"receiver_chat_id": 1}, "logging": {"dirs": "info"},
            "database": {"host": "unused", "port": 5432, "user": "", "password": "", "database": ""},
            "redis": {"host": "unused", "port": 6379},
            "yt_dlp": {"max_file_size": 1_000_000}, "yt_toolkit": {"url": "https://example.test"},
            "download": {"node_token": "test", "tls": {"ca_cert_path": "", "cert_path": "", "key_path": ""}},
            "telegram_bot_api": {"url": "https://example.test"}
        }))
        .unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        let formatter = Arc::new(ErrorFormatter::new("test"));
        let messenger = Arc::new(RecordingMessenger(events.clone()));
        let stats = Stats::new(
            formatter.clone(),
            messenger.clone(),
            Arc::new(downloaded_media::GetStats::new(tx_manager.clone())),
            Arc::new(node_router::GetStats::new(router)),
            redis.queue.clone(),
        );

        Self {
            events,
            open_menu: Arc::new(OpenMenu::new(Arc::new(config), formatter.clone(), messenger, Arc::new(stats))),
            formatter,
            fsm: Fsm::new(MemoryStorage::new(), StorageKey::new(1, CHAT_ID, CHAT_ID, None, None)),
            tx_manager,
            _postgres: postgres,
            _redis: redis,
        }
    }

    async fn chat(&self) -> Saved {
        let reader = self.tx_manager.chat_config_reader();
        Saved {
            config: reader.get(CHAT_ID).await.unwrap().unwrap(),
            exclude_domains: reader.get_exclude_domains(CHAT_ID).await.unwrap(),
        }
    }

    // The menu `OpenMenu` draws for the saved chat, to compare an interactor's result against.
    async fn saved_menu(&self, screen: Screen) -> String {
        let Saved { config, exclude_domains } = self.chat().await;
        self.open_menu
            .as_ref()
            .execute(OpenMenuInput {
                target: EDIT,
                screen,
                notice: None,
                chat_cfg: &config,
                exclude_domains: &exclude_domains,
                fsm: &self.fsm,
            })
            .await
            .unwrap();

        let edit = self.events.lock().unwrap().pop().unwrap();
        edit.strip_prefix("edit ").unwrap().to_owned()
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    async fn add(&self, host: &str) {
        let Saved { config, exclude_domains } = self.chat().await;
        AddMenuDomain::new(
            self.formatter.clone(),
            Arc::new(chat::AddExcludeDomain::new(self.tx_manager.clone())),
            self.open_menu.clone(),
        )
        .execute(AddMenuDomainInput {
            chat_id: CHAT_ID,
            host: Some(host),
            chat_cfg: &config,
            exclude_domains: &exclude_domains,
            fsm: &self.fsm,
        })
        .await
        .unwrap();
    }

    async fn remove(&self, domain: DomainKey) {
        let Saved { config, exclude_domains } = self.chat().await;
        RemoveMenuDomain::new(
            self.formatter.clone(),
            Arc::new(chat::RemoveExcludeDomain::new(self.tx_manager.clone())),
            self.open_menu.clone(),
        )
        .execute(RemoveMenuDomainInput {
            target: EDIT,
            domain,
            chat_cfg: &config,
            exclude_domains: &exclude_domains,
            fsm: &self.fsm,
        })
        .await
        .unwrap();
    }
}

fn domains(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("site{index}.example")).collect()
}

#[tokio::test]
async fn the_add_screen_arms_the_domain_prompt_and_another_screen_clears_it() {
    let fixture = Fixture::new(&[]).await;
    let Saved { config, exclude_domains } = fixture.chat().await;

    for screen in [Screen::DomainAdd, Screen::Domains] {
        fixture
            .open_menu
            .as_ref()
            .execute(OpenMenuInput {
                target: EDIT,
                screen,
                notice: None,
                chat_cfg: &config,
                exclude_domains: &exclude_domains,
                fsm: &fixture.fsm,
            })
            .await
            .unwrap();

        let expected = (screen == Screen::DomainAdd).then(|| MenuState::DomainInput.as_ref().into());
        assert_eq!(fixture.fsm.get_state().await.unwrap(), expected, "{screen:?}");
    }
}

#[tokio::test]
async fn removing_a_domain_deletes_that_domain_and_answers_before_editing() {
    let fixture = Fixture::new(&domains(2)).await;
    let before = fixture.chat().await.exclude_domains;

    // Not the first row, so removing by position 0 instead of by fingerprint fails.
    fixture.remove(DomainKey::new(&before.0[1])).await;

    let after = fixture.chat().await.exclude_domains;
    assert_eq!(after.0, [before.0[0].clone()]);

    let events = fixture.events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0], r#"answer Some("Domain removed")"#);
    assert_eq!(events[1], format!("edit {}", fixture.saved_menu(Screen::Domains).await));
}

#[tokio::test]
async fn removing_a_domain_that_already_left_changes_nothing() {
    let fixture = Fixture::new(&domains(2)).await;

    fixture.remove(DomainKey::new("gone.example")).await;

    let after = fixture.chat().await.exclude_domains;
    assert_eq!(after.0.len(), 2);

    let events = fixture.events();
    assert_eq!(events[0], "answer None");
    assert_eq!(events[1], format!("edit {}", fixture.saved_menu(Screen::Domains).await));
}

#[tokio::test]
async fn adding_a_domain_saves_it_and_heads_the_menu_with_the_notice() {
    let fixture = Fixture::new(&[]).await;

    fixture.add("new.example").await;

    let after = fixture.chat().await.exclude_domains;
    assert_eq!(after.0, ["new.example"]);

    let sent = fixture.events().remove(0);
    let menu = fixture.saved_menu(Screen::Domains).await;
    assert_eq!(sent, format!("send Domain new.example added\n\n{menu}"));
}

#[tokio::test]
async fn adding_a_domain_already_in_the_list_is_refused() {
    let fixture = Fixture::new(&domains(2)).await;

    fixture.add("site0.example").await;

    let after = fixture.chat().await.exclude_domains;
    assert_eq!(after.0, domains(2));

    let sent = fixture.events().remove(0);
    let notice = t!("exclude_domain.already_in_list", locale = "en");
    let menu = fixture.saved_menu(Screen::Domains).await;
    assert_eq!(sent, format!("send {notice}\n\n{menu}"));
}

#[tokio::test]
async fn adding_a_domain_to_a_full_list_is_refused() {
    let fixture = Fixture::new(&domains(MAX_EXCLUDE_DOMAINS)).await;

    fixture.add("new.example").await;

    let after = fixture.chat().await.exclude_domains;
    assert_eq!(after.0.len(), MAX_EXCLUDE_DOMAINS);
    assert!(!after.0.contains(&"new.example".to_owned()));

    let sent = fixture.events().remove(0);
    let notice = t!("exclude_domain.limit_reached", locale = "en");
    let menu = fixture.saved_menu(Screen::Domains).await;
    assert_eq!(sent, format!("send {notice}\n\n{menu}"));
}

#[tokio::test]
async fn choosing_a_language_saves_it_and_redraws_the_menu_in_it() {
    let fixture = Fixture::new(&[]).await;
    let Saved { config, exclude_domains } = fixture.chat().await;

    SetMenuLanguage::new(
        fixture.formatter.clone(),
        Arc::new(chat::UpdateChatConfig::new(fixture.tx_manager.clone())),
        fixture.open_menu.clone(),
    )
    .execute(SetMenuLanguageInput {
        target: EDIT,
        locale: Locale::Ru,
        chat_cfg: &config,
        exclude_domains: &exclude_domains,
        fsm: &fixture.fsm,
    })
    .await
    .unwrap();

    assert_eq!(fixture.chat().await.config.locale(), Locale::Ru);

    let edit = fixture.events().remove(1);
    assert_eq!(edit, format!("edit {}", fixture.saved_menu(Screen::Language).await));
}

#[tokio::test]
async fn toggling_link_visibility_saves_it_and_redraws_the_settings() {
    let fixture = Fixture::new(&[]).await;
    let Saved { config, exclude_domains } = fixture.chat().await;
    assert!(!config.link_is_visible);

    SetMenuLinkVisibility::new(
        fixture.formatter.clone(),
        Arc::new(chat::UpdateChatConfig::new(fixture.tx_manager.clone())),
        fixture.open_menu.clone(),
    )
    .execute(SetMenuLinkVisibilityInput {
        target: EDIT,
        link_is_visible: true,
        chat_cfg: &config,
        exclude_domains: &exclude_domains,
        fsm: &fixture.fsm,
    })
    .await
    .unwrap();

    assert!(fixture.chat().await.config.link_is_visible);

    let edit = fixture.events().remove(1);
    assert_eq!(edit, format!("edit {}", fixture.saved_menu(Screen::Settings).await));
}
