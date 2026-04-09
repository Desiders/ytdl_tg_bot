use froodi::{
    async_impl::Container,
    async_registry, instance, registry,
    DefaultScope::{App, Request},
    Inject, InstantiateErrorKind,
};
use reqwest::Client;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::sync::{Arc, Mutex};
use telers::Bot;
use tracing::{error, info};
use uuid::ContextV7;

use crate::{
    config::{Config, DatabaseConfig, RandomCmdConfig, TimeoutsConfig, TrackingParamsConfig, YtToolkitConfig},
    database::TxManager,
    interactors::{chat, download::media, downloaded_media, get_media, node_router, send_media},
    services::node_router::{DownloaderClusterConfig, DownloaderServiceTarget, DownloaderTlsConfig, NodeRouter},
};

#[allow(clippy::too_many_lines)]
pub(super) fn init(bot: Bot, cfg: Config) -> Container {
    let sync_registry = registry! {
        scope(App) [
            provide(instance(bot)),
            provide(instance(cfg.clone())),
            provide(instance(cfg.bot)),
            provide(instance(cfg.chat)),
            provide(instance(cfg.timeouts)),
            provide(instance(cfg.blacklisted)),
            provide(instance(cfg.logging)),
            provide(instance(cfg.database)),
            provide(instance(cfg.yt_dlp)),
            provide(instance(cfg.yt_toolkit)),
            provide(instance(cfg.telegram_bot_api)),
            provide(instance(cfg.domains_with_reactions)),
            provide(instance(cfg.random_cmd)),
            provide(instance(cfg.replace_domains)),
            provide(instance(cfg.tracking_params)),

            provide(|| Ok(Mutex::new(ContextV7::new()))),
            provide(|| Ok(Client::new())),
            provide(|| Ok(chat::SaveChat {})),
            provide(|| Ok(chat::AddExcludeDomain {})),
            provide(|| Ok(chat::RemoveExcludeDomain {})),
            provide(|| Ok(chat::UpdateChatConfig {})),
            provide(|| Ok(downloaded_media::AddVideo {})),
            provide(|| Ok(downloaded_media::AddAudio {})),
            provide(|| Ok(downloaded_media::GetStats {})),
            provide(|| Ok(DownloaderServiceTarget::from_env())),

            provide(|Inject(random_cfg): Inject<RandomCmdConfig>| Ok(downloaded_media::GetRandomVideo { random_cfg })),
            provide(|Inject(random_cfg): Inject<RandomCmdConfig>| Ok(downloaded_media::GetRandomAudio { random_cfg })),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(send_media::upload::SendVideo { bot, timeouts_cfg })
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(send_media::upload::SendAudio { bot, timeouts_cfg })
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(send_media::id::EditVideo { bot, timeouts_cfg })
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(send_media::id::EditAudio { bot, timeouts_cfg })
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(send_media::id::SendVideo { bot, timeouts_cfg })
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(send_media::id::SendAudio { bot, timeouts_cfg })
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(send_media::id::SendVideoPlaylist { bot, timeouts_cfg })
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(send_media::id::SendAudioPlaylist { bot, timeouts_cfg })
            ),
            provide(|
                Inject(client): Inject<Client>,
                Inject(yt_toolkit_cfg): Inject<YtToolkitConfig>| Ok(get_media::GetShortMediaByURL { client, yt_toolkit_cfg })
            ),
            provide(|
                Inject(client): Inject<Client>,
                Inject(yt_toolkit_cfg): Inject<YtToolkitConfig>| Ok(get_media::SearchMediaInfo { client, yt_toolkit_cfg })
            ),

            provide(
                |Inject(cfg): Inject<Config>, Inject(service_target): Inject<DownloaderServiceTarget>| {
                    let downloader_cfg = DownloaderClusterConfig {
                        token: cfg.download.token.clone(),
                        tls: DownloaderTlsConfig {
                            ca_cert_path: cfg.download.tls.ca_cert_path.clone(),
                            cert_path: cfg.download.tls.cert_path.clone(),
                            key_path: cfg.download.tls.key_path.clone(),
                        },
                    };
                    Ok(NodeRouter::new(&downloader_cfg, cfg.yt_dlp.max_file_size, service_target))
                },
            ),
            provide(|Inject(node_router): Inject<NodeRouter>| Ok(node_router::GetStats { node_router })),
            provide(|
                Inject(node_router): Inject<NodeRouter>,
                Inject(tracking_params_cfg): Inject<TrackingParamsConfig>| Ok(get_media::GetUncachedVideoByURL { node_router, tracking_params_cfg })
            ),
            provide(|
                Inject(node_router): Inject<NodeRouter>,
                Inject(tracking_params_cfg): Inject<TrackingParamsConfig>| Ok(get_media::GetVideoByURL { node_router, tracking_params_cfg })
            ),
            provide(|
                Inject(node_router): Inject<NodeRouter>,
                Inject(tracking_params_cfg): Inject<TrackingParamsConfig>| Ok(get_media::GetAudioByURL { node_router, tracking_params_cfg })
            ),
            provide(|Inject(node_router): Inject<NodeRouter>| Ok(media::DownloadVideo { node_router })),
            provide(|Inject(node_router): Inject<NodeRouter>| Ok(media::DownloadAudio { node_router })),
            provide(|Inject(node_router): Inject<NodeRouter>|  Ok(media::DownloadVideoPlaylist { node_router })),
            provide(|Inject(node_router): Inject<NodeRouter>|  Ok(media::DownloadAudioPlaylist { node_router })),
        ],
    };
    let registry_with_sync = async_registry! {
        provide(
            App,
            |Inject(database_cfg): Inject<DatabaseConfig>| async move {
                let mut options = ConnectOptions::new(database_cfg.get_postgres_url());
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
            finalizer = |database_conn: Arc<DatabaseConnection>| async move {
                match database_conn.close_by_ref().await {
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
        extend(sync_registry),
    };

    Container::new(registry_with_sync)
}
