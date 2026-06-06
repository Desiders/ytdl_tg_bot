use froodi::{
    async_impl::{Container, RegistryWithSync},
    async_registry, boxed, instance, registry,
    DefaultScope::{App, Request},
    Inject, InstantiateErrorKind, Registry,
};
use redis::aio::ConnectionManager;
use reqwest::Client;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use telers::Bot;
use tracing::{error, info};
use uuid::ContextV7;

use crate::{
    config::{BotConfig, Config, DatabaseConfig, DownloadConfig, RedisConfig, TimeoutsConfig, YtDlpConfig},
    database::{SeaOrmTxManager, TxManager, TxManagerFactories},
    interactors::{audio, chosen_inline, config, enqueue_download, inline_query, lang, photo, start, stats, video},
    services::{
        chat,
        download::media,
        downloaded_media, get_media,
        messenger::telegram::TelegramMessenger,
        node_router::{self, DownloaderServiceTarget, NodeRouter},
        queue::RedisJobQueue,
        send_media,
    },
    utils::ErrorFormatter,
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
            provide(instance(cfg.redis.clone())),
            provide(instance(cfg.redis.queue)),
            provide(instance(cfg.yt_dlp)),
            provide(instance(cfg.yt_toolkit)),
            provide(instance(cfg.download)),
            provide(instance(cfg.telegram_bot_api)),
            provide(instance(cfg.domains_with_reactions)),
            provide(instance(cfg.random_cmd)),
            provide(instance(cfg.tracking_params)),
        ]
    }
}

pub(super) fn tg_messenger_registry(bot: Bot, cfg_registry: Registry) -> Registry {
    registry! {
        scope(App) [
            provide(instance(bot)),
            provide(|Inject(cfg): Inject<BotConfig>| Ok(ErrorFormatter::new(cfg.token.clone()))),
            provide(|Inject(bot): Inject<Bot>, Inject(error_formatter): Inject<ErrorFormatter>, Inject(cfg): Inject<TimeoutsConfig>| {
                Ok(TelegramMessenger::new(bot, error_formatter, cfg))
            }),
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
                options
                    .max_connections(cfg.max_connections)
                    .acquire_timeout(Duration::from_secs(cfg.acquire_timeout_secs))
                    .connect_timeout(Duration::from_secs(cfg.connect_timeout_secs))
                    .sqlx_logging(false);

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
        provide(App, || async move { Ok(TxManagerFactories::default()) }),
        provide(
            Request,
            |Inject(pool): Inject<DatabaseConnection>, Inject(factories): Inject<TxManagerFactories>| async move {
                Ok(boxed!(SeaOrmTxManager::new(pool, factories); TxManager))
            },
        ),
        extend(cfg_registry),
    }
}

pub(super) fn queue_registry(cfg_registry: Registry) -> RegistryWithSync {
    async_registry! {
        provide(
            App,
            |Inject(cfg): Inject<RedisConfig>| async move {
                match redis::Client::open(cfg.get_url()) {
                    Ok(client) => Ok(client),
                    Err(err) => {
                        error!(%err, "Open Redis client error");
                        Err(InstantiateErrorKind::Custom(err.into()))
                    }
                }
            },
        ),
        provide(
            App,
            |Inject(client): Inject<redis::Client>| async move {
                match ConnectionManager::new((*client).clone()).await {
                    Ok(conn) => Ok(conn),
                    Err(err) => {
                        error!(%err, "Create Redis conn error");
                        Err(InstantiateErrorKind::Custom(err.into()))
                    }
                }
            },
        ),
        provide(
            App,
            |Inject(conn): Inject<ConnectionManager>, Inject(cfg)| async move { Ok(RedisJobQueue::new((*conn).clone(), cfg)) },
        ),
        extend(cfg_registry),
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn interactors_registry<Messenger>(
    cfg_registry: Registry,
    tg_messenger_registry: Registry,
    node_router_registry: Registry,
) -> RegistryWithSync
where
    Messenger: Send + Sync + 'static,
{
    async_registry! {
        scope(App) [
            provide(|| async move { Ok(Mutex::new(ContextV7::new())) }),
            provide(|| async move { Ok(Client::new()) }),

            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::upload::SendVideo::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::upload::SendAudio::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::upload::SendPhoto::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::upload::SendPhotoUrl::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::id::EditVideo::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::id::EditAudio::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::id::SendVideo::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::id::SendAudio::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::id::SendPhoto::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::id::EditPhoto::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::id::SendVideoPlaylist::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::id::SendAudioPlaylist::new(messenger)) }),
            provide(|Inject(messenger): Inject<Messenger>| async move { Ok(send_media::id::SendPhotoPlaylist::new(messenger)) }),

            provide(|
                Inject(cfg),
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>| async move {
                    Ok(start::Start::new(cfg, error_formatter, messenger))
                }
            ),
            provide(|
                Inject(client),
                Inject(cfg)| async move { Ok(get_media::GetShortMediaByURL::new(client, cfg)) }
            ),
            provide(|
                Inject(client),
                Inject(cfg)| async move { Ok(get_media::SearchMediaInfo::new(client, cfg)) }
            ),

            provide(|Inject(node_router)| async move { Ok(node_router::GetStats::new(node_router)) }),
            provide(|Inject(node_router)| async move { Ok(media::DownloadVideo::new(node_router)) }),
            provide(|Inject(node_router)| async move { Ok(media::DownloadAudio::new(node_router)) }),
            provide(|Inject(node_router)| async move { Ok(media::DownloadPhoto::new(node_router)) }),
            provide(|Inject(node_router)| async move { Ok(media::DownloadVideoPlaylist::new(node_router)) }),
            provide(|Inject(node_router)| async move { Ok(media::DownloadAudioPlaylist::new(node_router)) }),
            provide(|Inject(node_router)| async move { Ok(media::DownloadPhotoPlaylist::new(node_router)) }),

            provide(|
                Inject(messenger): Inject<Messenger>,
                Inject(queue)| async move {
                    Ok(enqueue_download::EnqueueCommandDownload::new(messenger, queue))
                }
            ),
            provide(|
                Inject(messenger): Inject<Messenger>,
                Inject(queue)| async move {
                    Ok(enqueue_download::EnqueueInlineDownload::new(messenger, queue))
                }
            ),

            provide(|
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(get_basic_info_media)| async move {
                    Ok(inline_query::SelectByUrl::new(error_formatter, messenger, get_basic_info_media))
                }
            ),
            provide(|
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(get_basic_info_media)| async move {
                    Ok(inline_query::SelectByText::new(error_formatter, messenger, get_basic_info_media))
                }
            ),
        ],

        scope(Request) [
            provide(|Inject(tx_manager)| async move { Ok(chat::SaveChat::new(tx_manager)) }),
            provide(|Inject(tx_manager)| async move { Ok(chat::GetChatConfig::new(tx_manager)) }),
            provide(|Inject(tx_manager)| async move { Ok(chat::AddExcludeDomain::new(tx_manager)) }),
            provide(|Inject(tx_manager)| async move { Ok(chat::RemoveExcludeDomain::new(tx_manager)) }),
            provide(|Inject(tx_manager)| async move { Ok(chat::UpdateChatConfig::new(tx_manager)) }),
            provide(|Inject(tx_manager)| async move { Ok(downloaded_media::AddVideo::new(tx_manager)) }),
            provide(|Inject(tx_manager)| async move { Ok(downloaded_media::AddAudio::new(tx_manager)) }),
            provide(|Inject(tx_manager)| async move { Ok(downloaded_media::AddPhoto::new(tx_manager)) }),
            provide(|Inject(tx_manager)| async move { Ok(downloaded_media::GetStats::new(tx_manager)) }),
            provide(|Inject(cfg), Inject(tx_manager)| async move { Ok(downloaded_media::GetRandomVideo::new(cfg, tx_manager)) }),
            provide(|Inject(cfg), Inject(tx_manager)| async move { Ok(downloaded_media::GetRandomAudio::new(cfg, tx_manager)) }),

            provide(|
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(get_media_stats),
                Inject(get_node_stats),
                Inject(queue)| async move {
                    Ok(stats::Stats::new(error_formatter, messenger, get_media_stats, get_node_stats, queue))
                }
            ),
            provide(|
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(update_chat_cfg)| async move {
                    Ok(config::ChangeLinkVisibility::new(error_formatter, messenger, update_chat_cfg))
                }
            ),
            provide(|
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(update_chat_cfg)| async move {
                    Ok(lang::Lang::new(error_formatter, messenger, update_chat_cfg))
                }
            ),
            provide(|
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(add_domain)| async move {
                    Ok(config::AddExcludeDomain::new(error_formatter, messenger, add_domain))
                }
            ),
            provide(|
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(remove_domain)| async move {
                    Ok(config::RemoveExcludeDomain::new(error_formatter, messenger, remove_domain))
                }
            ),

            provide(|
                Inject(node_router),
                Inject(cfg),
                Inject(tx_manager)| async move {
                    Ok(get_media::GetVideoByURL::new(node_router, cfg, tx_manager))
                }
            ),
            provide(|
                Inject(node_router),
                Inject(cfg),
                Inject(tx_manager)| async move {
                    Ok(get_media::GetAudioByURL::new(node_router, cfg, tx_manager))
                }
            ),
            provide(|
                Inject(node_router),
                Inject(cfg),
                Inject(tx_manager)| async move {
                    Ok(get_media::GetPhotoByURL::new(node_router, cfg, tx_manager))
                }
            ),

            provide(|
                Inject(cfg),
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(get_media),
                Inject(download_playlist),
                Inject(upload_media): Inject<send_media::upload::SendVideo<Messenger>>,
                Inject(send_media_by_id): Inject<send_media::id::SendVideo<Messenger>>,
                Inject(send_playlist): Inject<send_media::id::SendVideoPlaylist<Messenger>>,
                Inject(add_downloaded_media)| async move {
                    Ok(video::Download::new(
                        cfg, error_formatter, messenger, get_media,
                        download_playlist,
                        upload_media, send_media_by_id, send_playlist, add_downloaded_media,
                    ))
                }
            ),
            provide(|
                Inject(cfg),
                Inject(error_formatter),
                Inject(get_media),
                Inject(download_playlist),
                Inject(upload_media): Inject<send_media::upload::SendVideo<Messenger>>,
                Inject(send_media_by_id): Inject<send_media::id::SendVideo<Messenger>>,
                Inject(send_playlist): Inject<send_media::id::SendVideoPlaylist<Messenger>>,
                Inject(add_downloaded_media)| async move {
                    Ok(video::DownloadQuiet::new(
                        cfg, error_formatter, get_media,
                        download_playlist,
                        upload_media, send_media_by_id, send_playlist, add_downloaded_media,
                    ))
                }
            ),
            provide(|
                Inject(error_formatter),
                Inject(get_media),
                Inject(send_playlist): Inject<send_media::id::SendVideoPlaylist<Messenger>>| async move {
                    Ok(video::Random::new(error_formatter, get_media, send_playlist))
                }
            ),
            provide(|
                Inject(cfg),
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(get_media),
                Inject(download_playlist),
                Inject(upload_media): Inject<send_media::upload::SendAudio<Messenger>>,
                Inject(send_media_by_id): Inject<send_media::id::SendAudio<Messenger>>,
                Inject(send_playlist): Inject<send_media::id::SendAudioPlaylist<Messenger>>,
                Inject(add_downloaded_media)| async move {
                    Ok(audio::Download::new(
                        cfg, error_formatter, messenger, get_media,
                        download_playlist,
                        upload_media, send_media_by_id, send_playlist, add_downloaded_media,
                    ))
                }
            ),
            provide(|
                Inject(error_formatter),
                Inject(get_media),
                Inject(send_playlist): Inject<send_media::id::SendAudioPlaylist<Messenger>>| async move {
                    Ok(audio::Random::new(error_formatter, get_media, send_playlist))
                }
            ),
            provide(|
                Inject(cfg),
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(get_media),
                Inject(upload_media): Inject<send_media::upload::SendPhotoUrl<Messenger>>,
                Inject(send_media_by_id): Inject<send_media::id::SendPhoto<Messenger>>,
                Inject(send_playlist): Inject<send_media::id::SendPhotoPlaylist<Messenger>>,
                Inject(add_downloaded_media)| async move {
                    Ok(photo::Download::new(
                        cfg, error_formatter, messenger, get_media,
                        upload_media, send_media_by_id, send_playlist, add_downloaded_media,
                    ))
                }
            ),
            provide(|
                Inject(cfg),
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(get_media),
                Inject(download_media),
                Inject(upload_media): Inject<send_media::upload::SendVideo<Messenger>>,
                Inject(edit_media_by_id): Inject<send_media::id::EditVideo<Messenger>>,
                Inject(add_downloaded_media)| async move {
                    Ok(chosen_inline::DownloadVideo::new(
                        cfg, error_formatter, messenger, get_media, download_media,
                        upload_media, edit_media_by_id, add_downloaded_media,
                    ))
                }
            ),
            provide(|
                Inject(cfg),
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(get_media),
                Inject(download_media),
                Inject(upload_media): Inject<send_media::upload::SendAudio<Messenger>>,
                Inject(edit_media_by_id): Inject<send_media::id::EditAudio<Messenger>>,
                Inject(add_downloaded_media)| async move {
                    Ok(chosen_inline::DownloadAudio::new(
                        cfg, error_formatter, messenger, get_media, download_media,
                        upload_media, edit_media_by_id, add_downloaded_media,
                    ))
                }
            ),
            provide(|
                Inject(cfg),
                Inject(error_formatter),
                Inject(messenger): Inject<Messenger>,
                Inject(get_media),
                Inject(upload_media): Inject<send_media::upload::SendPhotoUrl<Messenger>>,
                Inject(edit_media_by_id): Inject<send_media::id::EditPhoto<Messenger>>,
                Inject(add_downloaded_media)| async move {
                    Ok(chosen_inline::DownloadPhoto::new(
                        cfg, error_formatter, messenger, get_media,
                        upload_media, edit_media_by_id, add_downloaded_media,
                    ))
                }
            ),
        ],
        extend(cfg_registry, tg_messenger_registry, node_router_registry),
    }
}

pub(super) fn init(
    interactors_registry: RegistryWithSync,
    database_registry: RegistryWithSync,
    queue_registry: RegistryWithSync,
) -> Container {
    let registry = async_registry! {
        extend(interactors_registry, database_registry, queue_registry),
    };
    Container::new(registry)
}
