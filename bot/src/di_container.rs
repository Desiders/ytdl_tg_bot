use froodi::{
    async_impl::{Container, RegistryWithSync},
    async_registry, instance, registry,
    DefaultScope::{App, Request},
    Inject, InstantiateErrorKind, Registry,
};
use reqwest::Client;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::sync::{Arc, Mutex};
use telers::Bot;
use tracing::{error, info};
use uuid::ContextV7;

use crate::{
    config::{
        BotConfig, Config, DatabaseConfig, DownloadConfig, RandomCmdConfig, TimeoutsConfig, TrackingParamsConfig, YtDlpConfig,
        YtToolkitConfig,
    },
    database::TxManager,
    interactors::{chat, download::media, downloaded_media, get_media, node_router, send_media},
    services::{
        messenger::telegram::TelegramMessenger,
        node_router::{DownloaderServiceTarget, NodeRouter},
    },
    utils::ErrorMessageFormatter,
};

pub(super) fn cfg_registry(cfg: Config) -> Registry {
    registry! {
        scope(App) [
            provide(instance(cfg.clone())),
            provide(instance(cfg.bot)),
            provide(instance(cfg.chat)),
            provide(instance(cfg.timeouts)),
            provide(instance(cfg.blacklisted)),
            provide(instance(cfg.logging)),
            provide(instance(cfg.database)),
            provide(instance(cfg.yt_dlp)),
            provide(instance(cfg.yt_toolkit)),
            provide(instance(cfg.download)),
            provide(instance(cfg.telegram_bot_api)),
            provide(instance(cfg.domains_with_reactions)),
            provide(instance(cfg.random_cmd)),
            provide(instance(cfg.replace_domains)),
            provide(instance(cfg.tracking_params)),
        ]
    }
}

pub(super) fn tg_messenger_registry(bot: Bot, cfg_registry: Registry) -> Registry {
    registry! {
        scope(App) [
            provide(instance(bot)),
            provide(|Inject(cfg): Inject<BotConfig>| Ok(ErrorMessageFormatter::new(cfg.token.clone()))),
            provide(|Inject(bot): Inject<Bot>, Inject(cfg): Inject<TimeoutsConfig>| Ok(TelegramMessenger::new(bot, cfg))),
        ],
        extend(cfg_registry),
    }
}

pub(super) fn node_router_registry(cfg_registry: Registry) -> Registry {
    registry! {
        scope(App) [
            provide(|| Ok(DownloaderServiceTarget::from_env())),
            provide(|
                Inject(downloader_cfg): Inject<DownloadConfig>,
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(service_target): Inject<DownloaderServiceTarget>| {
                    Ok(NodeRouter::new(&(*downloader_cfg).clone().into(), yt_dlp_cfg.max_file_size, service_target))
                },
            ),
        ],
        extend(cfg_registry),
    }
}

pub(super) fn database_registry(cfg_registry: Registry) -> RegistryWithSync {
    async_registry! {
        provide(
            App,
            |Inject(cfg): Inject<DatabaseConfig>| async move {
                let mut options = ConnectOptions::new(cfg.get_postgres_url());
                options.sqlx_logging(false);

                match Database::connect(options).await {
                    Ok(database_conn) => {
                        info!("Database conn created");
                        Ok(database_conn)
                    }
                    Err(err) => {
                        error!(%err, "Create database conn error");
                        Err(InstantiateErrorKind::Custom(err.into()))
                    }
                }
            },
            finalizer = |conn: Arc<DatabaseConnection>| async move {
                match conn.close_by_ref().await {
                    Ok(()) => {
                        info!("Database conn closed");
                    },
                    Err(err) => {
                        error!(%err, "Close database conn error");
                    },
                }
            },
         ),
        provide(
            Request,
            |Inject(pool): Inject<DatabaseConnection>| async move { Ok(TxManager::new(pool)) },
        ),
        extend(cfg_registry),
    }
}

pub(super) fn interactors_registry<Messenger>(
    cfg_registry: Registry,
    tg_messenger_registry: Registry,
    node_router_registry: Registry,
) -> Registry
where
    Messenger: Send + Sync + 'static,
{
    registry! {
        scope(App) [
            provide(|| Ok(Mutex::new(ContextV7::new()))),
            provide(|| Ok(Client::new())),

            provide(|| Ok(chat::SaveChat {})),
            provide(|| Ok(chat::AddExcludeDomain {})),
            provide(|| Ok(chat::RemoveExcludeDomain {})),
            provide(|| Ok(chat::UpdateChatConfig {})),
            provide(|| Ok(downloaded_media::AddVideo {})),
            provide(|| Ok(downloaded_media::AddAudio {})),
            provide(|| Ok(downloaded_media::GetStats {})),

            provide(|Inject(cfg): Inject<RandomCmdConfig>| Ok(downloaded_media::GetRandomVideo { cfg })),
            provide(|Inject(cfg): Inject<RandomCmdConfig>| Ok(downloaded_media::GetRandomAudio { cfg })),
            provide(|Inject(messenger): Inject<Messenger>| Ok(send_media::upload::SendVideo { messenger })),
            provide(|Inject(messenger): Inject<Messenger>| Ok(send_media::upload::SendAudio { messenger })),
            provide(|Inject(messenger): Inject<Messenger>| Ok(send_media::id::EditVideo { messenger })),
            provide(|Inject(messenger): Inject<Messenger>| Ok(send_media::id::EditAudio { messenger })),
            provide(|Inject(messenger): Inject<Messenger>| Ok(send_media::id::SendVideo { messenger })),
            provide(|Inject(messenger): Inject<Messenger>| Ok(send_media::id::SendAudio { messenger })),
            provide(|Inject(messenger): Inject<Messenger>| Ok(send_media::id::SendVideoPlaylist { messenger })),
            provide(|Inject(messenger): Inject<Messenger>| Ok(send_media::id::SendAudioPlaylist { messenger })),
            provide(|
                Inject(client): Inject<Client>,
                Inject(cfg): Inject<YtToolkitConfig>| Ok(get_media::GetShortMediaByURL { client, cfg })
            ),
            provide(|
                Inject(client): Inject<Client>,
                Inject(cfg): Inject<YtToolkitConfig>| Ok(get_media::SearchMediaInfo { client, cfg })
            ),

            provide(|Inject(node_router): Inject<NodeRouter>| Ok(node_router::GetStats { node_router })),
            provide(|
                Inject(node_router): Inject<NodeRouter>,
                Inject(cfg): Inject<TrackingParamsConfig>| Ok(get_media::GetUncachedVideoByURL { node_router, cfg })
            ),
            provide(|
                Inject(node_router): Inject<NodeRouter>,
                Inject(cfg): Inject<TrackingParamsConfig>| Ok(get_media::GetVideoByURL { node_router, cfg })
            ),
            provide(|
                Inject(node_router): Inject<NodeRouter>,
                Inject(cfg): Inject<TrackingParamsConfig>| Ok(get_media::GetAudioByURL { node_router, cfg })
            ),
            provide(|Inject(node_router): Inject<NodeRouter>| Ok(media::DownloadVideo { node_router })),
            provide(|Inject(node_router): Inject<NodeRouter>| Ok(media::DownloadAudio { node_router })),
            provide(|Inject(node_router): Inject<NodeRouter>|  Ok(media::DownloadVideoPlaylist { node_router })),
            provide(|Inject(node_router): Inject<NodeRouter>|  Ok(media::DownloadAudioPlaylist { node_router })),
        ],
        extend(cfg_registry, tg_messenger_registry, node_router_registry),
    }
}

pub(super) fn init(
    cfg_registry: Registry,
    tg_messenger_registry: Registry,
    node_router_registry: Registry,
    interactors_registry: Registry,
    database_registry: RegistryWithSync,
) -> Container {
    let registry = async_registry! {
        extend(cfg_registry, tg_messenger_registry, node_router_registry, interactors_registry, database_registry),
    };
    Container::new(registry)
}
