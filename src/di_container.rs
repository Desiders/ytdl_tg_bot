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
    config::{Config, DatabaseConfig, RandomCmdConfig, TimeoutsConfig, YtDlpConfig, YtPotProviderConfig, YtToolkitConfig},
    database::TxManager,
    entities::Cookies,
    interactors::{
        download::{DownloadAudio, DownloadAudioPlaylist, DownloadVideo, DownloadVideoPlaylist},
        send_media::{
            EditAudioById, EditVideoById, SendAudioById, SendAudioInFS, SendAudioPlaylistById, SendVideoById, SendVideoInFS,
            SendVideoPlaylistById,
        },
        AddDownloadedAudio, AddDownloadedVideo, GetAudioByURL, GetRandomDownloadedAudio, GetRandomDownloadedVideo, GetShortMediaByURLInfo,
        GetUncachedVideoByURL, GetVideoByURL, SaveChat, SearchMediaInfo,
    },
};

pub(super) fn init(bot: Bot, config: Config, cookies: Cookies) -> Container {
    let sync_registry = registry! {
        scope(App) [
            provide(instance(bot)),
            provide(instance(cookies)),
            provide(instance(config.bot)),
            provide(instance(config.chat)),
            provide(instance(config.timeouts)),
            provide(instance(config.blacklisted)),
            provide(instance(config.logging)),
            provide(instance(config.database)),
            provide(instance(config.yt_dlp)),
            provide(instance(config.yt_toolkit)),
            provide(instance(config.yt_pot_provider)),
            provide(instance(config.telegram_bot_api)),
            provide(instance(config.domains_with_reactions)),
            provide(instance(config.random_cmd)),
            provide(instance(config.replace_domains)),
            provide(instance(config.tracking_params)),

            provide(|| Ok(Mutex::new(ContextV7::new()))),
            provide(|| Ok(Client::new())),
            provide(|| Ok(SaveChat::new())),
            provide(|| Ok(AddDownloadedVideo::new())),
            provide(|| Ok(AddDownloadedAudio::new())),

            provide(|Inject(random_cfg): Inject<RandomCmdConfig>| Ok(GetRandomDownloadedVideo::new(random_cfg))),
            provide(|Inject(random_cfg): Inject<RandomCmdConfig>| Ok(GetRandomDownloadedAudio::new(random_cfg))),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(EditVideoById::new(bot, timeouts_cfg))
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(EditAudioById::new(bot, timeouts_cfg))
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(SendVideoById::new(bot, timeouts_cfg))
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(SendAudioById::new(bot, timeouts_cfg))
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(SendVideoInFS::new(bot, timeouts_cfg))
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(SendAudioInFS::new(bot, timeouts_cfg))
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(SendVideoPlaylistById::new(bot, timeouts_cfg))
            ),
            provide(|
                Inject(bot): Inject<Bot>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(SendAudioPlaylistById::new(bot, timeouts_cfg))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>| Ok(GetUncachedVideoByURL::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>| Ok(GetVideoByURL::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>| Ok(GetAudioByURL::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies))
            ),
            provide(
                |Inject(client): Inject<Client>,
                Inject(yt_toolkit_cfg): Inject<YtToolkitConfig>| Ok(GetShortMediaByURLInfo::new(client, yt_toolkit_cfg))
            ),
            provide(|
                Inject(client): Inject<Client>,
                Inject(yt_toolkit_cfg): Inject<YtToolkitConfig>| Ok(SearchMediaInfo::new(client, yt_toolkit_cfg))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(DownloadVideo::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies, timeouts_cfg))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(DownloadVideoPlaylist::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies, timeouts_cfg))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(DownloadAudio::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies, timeouts_cfg))
            ),
            provide(|
                Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
                Inject(yt_pot_provider_cfg): Inject<YtPotProviderConfig>,
                Inject(cookies): Inject<Cookies>,
                Inject(timeouts_cfg): Inject<TimeoutsConfig>| Ok(DownloadAudioPlaylist::new(yt_dlp_cfg, yt_pot_provider_cfg, cookies, timeouts_cfg))
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
