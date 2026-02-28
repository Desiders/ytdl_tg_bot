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
    config::{
        Config, DatabaseConfig, RandomCmdConfig, TimeoutsConfig, TrackingParamsConfig, YtDlpConfig, YtPotProviderConfig, YtToolkitConfig,
    },
    database::TxManager,
    entities::Cookies,
    interactors::{chat, download::media, downloaded_media, get_media, send_media},
};

pub(super) fn init(bot: Bot, cfg: Config, cookies: Cookies) -> Container {
    let sync_registry = registry! {
        scope(App) [
            provide(instance(bot)),
            provide(instance(cookies)),
            provide(instance(cfg.clone())),
            provide(instance(cfg.bot)),
            provide(instance(cfg.chat)),
            provide(instance(cfg.timeouts)),
            provide(instance(cfg.blacklisted)),
            provide(instance(cfg.logging)),
            provide(instance(cfg.database)),
            provide(instance(cfg.yt_dlp)),
            provide(instance(cfg.yt_toolkit)),
            provide(instance(cfg.yt_pot_provider)),
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

            provide(|Inject(random_cfg): Inject<RandomCmdConfig>| Ok(downloaded_media::GetRandomVideo { random_cfg })),
            provide(|Inject(random_cfg): Inject<RandomCmdConfig>| Ok(downloaded_media::GetRandomAudio { random_cfg })),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(send_media::fs::SendVideo { bot, timeouts_cfg })
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(send_media::fs::SendAudio { bot, timeouts_cfg })
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
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(tracking_params_cfg): Inject<TrackingParamsConfig>,
                Inject(cookies): Inject<Cookies>| Ok(get_media::GetUncachedVideoByURL { yt_dlp_cfg, yt_pot_provider_cfg, tracking_params_cfg, cookies })
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(tracking_params_cfg): Inject<TrackingParamsConfig>,
                Inject(cookies): Inject<Cookies>| Ok(get_media::GetVideoByURL { yt_dlp_cfg, yt_pot_provider_cfg, tracking_params_cfg, cookies })
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(tracking_params_cfg): Inject<TrackingParamsConfig>,
                Inject(cookies): Inject<Cookies>| Ok(get_media::GetAudioByURL { yt_dlp_cfg, yt_pot_provider_cfg, tracking_params_cfg, cookies })
            ),
            provide(
                |Inject(client): Inject<Client>,
                Inject(yt_toolkit_cfg): Inject<YtToolkitConfig>| Ok(get_media::GetShortMediaByURL { client, yt_toolkit_cfg })
            ),
            provide(|
                Inject(client): Inject<Client>,
                Inject(yt_toolkit_cfg): Inject<YtToolkitConfig>| Ok(get_media::SearchMediaInfo { client, yt_toolkit_cfg })
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>,
                Inject(cookies): Inject<Cookies>| Ok(media::DownloadVideo { yt_dlp_cfg, yt_pot_provider_cfg, timeouts_cfg, cookies })
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>,
                Inject(cookies): Inject<Cookies>| Ok(media::DownloadAudio { yt_dlp_cfg, yt_pot_provider_cfg, timeouts_cfg, cookies })
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>,
                Inject(cookies): Inject<Cookies>| Ok(media::DownloadVideoPlaylist { yt_dlp_cfg, yt_pot_provider_cfg, timeouts_cfg, cookies })
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>,
                Inject(cookies): Inject<Cookies>| Ok(media::DownloadAudioPlaylist { yt_dlp_cfg, yt_pot_provider_cfg, timeouts_cfg, cookies })
            ),
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
                        error!(%err, "Create database conn err");
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
                        error!(%err, "Close database conn err");
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
