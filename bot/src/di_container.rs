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
    interactors::{audio, chosen_inline, config, inline_query, start, stats, video},
    services::{
        chat,
        download::media,
        downloaded_media, get_media,
        messenger::telegram::TelegramMessenger,
        node_router::{self, DownloaderServiceTarget, NodeRouter},
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
                Inject(cfg): Inject<Config>,
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(messenger): Inject<Messenger>| {
                    Ok(start::Start {
                        cfg,
                        error_formatter,
                        messenger,
                    })
                }
            ),
            provide(|
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(messenger): Inject<Messenger>,
                Inject(get_media_stats): Inject<downloaded_media::GetStats>,
                Inject(get_node_stats): Inject<node_router::GetStats>| {
                    Ok(stats::Stats {
                        error_formatter,
                        messenger,
                        get_media_stats,
                        get_node_stats,
                    })
                }
            ),
            provide(|
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(messenger): Inject<Messenger>,
                Inject(update_chat_cfg): Inject<chat::UpdateChatConfig>| {
                    Ok(config::ChangeLinkVisibility {
                        error_formatter,
                        messenger,
                        update_chat_cfg,
                    })
                }
            ),
            provide(|
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(messenger): Inject<Messenger>,
                Inject(add_domain): Inject<chat::AddExcludeDomain>| {
                    Ok(config::AddExcludeDomain {
                        error_formatter,
                        messenger,
                        add_domain,
                    })
                }
            ),
            provide(|
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(messenger): Inject<Messenger>,
                Inject(remove_domain): Inject<chat::RemoveExcludeDomain>| {
                    Ok(config::RemoveExcludeDomain {
                        error_formatter,
                        messenger,
                        remove_domain,
                    })
                }
            ),
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

            provide(|
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(messenger): Inject<Messenger>,
                Inject(get_basic_info_media): Inject<get_media::GetShortMediaByURL>,
                Inject(get_media): Inject<get_media::GetUncachedVideoByURL>| {
                    Ok(inline_query::SelectByUrl {
                        error_formatter,
                        messenger,
                        get_basic_info_media,
                        get_media,
                    })
                }
            ),
            provide(|
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(messenger): Inject<Messenger>,
                Inject(get_basic_info_media): Inject<get_media::SearchMediaInfo>| {
                    Ok(inline_query::SelectByText {
                        error_formatter,
                        messenger,
                        get_basic_info_media,
                    })
                }
            ),
            provide(|
                Inject(cfg): Inject<Config>,
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(messenger): Inject<Messenger>,
                Inject(get_media): Inject<get_media::GetVideoByURL>,
                Inject(download_playlist): Inject<media::DownloadVideoPlaylist>,
                Inject(upload_media): Inject<send_media::upload::SendVideo<Messenger>>,
                Inject(send_media_by_id): Inject<send_media::id::SendVideo<Messenger>>,
                Inject(send_playlist): Inject<send_media::id::SendVideoPlaylist<Messenger>>,
                Inject(add_downloaded_media): Inject<downloaded_media::AddVideo>| {
                    Ok(video::Download {
                        cfg,
                        error_formatter,
                        messenger,
                        get_media,
                        download_playlist,
                        upload_media,
                        send_media_by_id,
                        send_playlist,
                        add_downloaded_media,
                    })
                }
            ),
            provide(|
                Inject(cfg): Inject<Config>,
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(get_media): Inject<get_media::GetVideoByURL>,
                Inject(download_playlist): Inject<media::DownloadVideoPlaylist>,
                Inject(upload_media): Inject<send_media::upload::SendVideo<Messenger>>,
                Inject(send_media_by_id): Inject<send_media::id::SendVideo<Messenger>>,
                Inject(send_playlist): Inject<send_media::id::SendVideoPlaylist<Messenger>>,
                Inject(add_downloaded_media): Inject<downloaded_media::AddVideo>| {
                    Ok(video::DownloadQuiet {
                        cfg,
                        error_formatter,
                        get_media,
                        download_playlist,
                        upload_media,
                        send_media_by_id,
                        send_playlist,
                        add_downloaded_media,
                    })
                }
            ),
            provide(|
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(get_media): Inject<downloaded_media::GetRandomVideo>,
                Inject(send_playlist): Inject<send_media::id::SendVideoPlaylist<Messenger>>| {
                    Ok(video::Random {
                        error_formatter,
                        get_media,
                        send_playlist,
                    })
                }
            ),
            provide(|
                Inject(cfg): Inject<Config>,
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(messenger): Inject<Messenger>,
                Inject(get_media): Inject<get_media::GetAudioByURL>,
                Inject(download_playlist): Inject<media::DownloadAudioPlaylist>,
                Inject(upload_media): Inject<send_media::upload::SendAudio<Messenger>>,
                Inject(send_media_by_id): Inject<send_media::id::SendAudio<Messenger>>,
                Inject(send_playlist): Inject<send_media::id::SendAudioPlaylist<Messenger>>,
                Inject(add_downloaded_media): Inject<downloaded_media::AddAudio>| {
                    Ok(audio::Download {
                        cfg,
                        error_formatter,
                        messenger,
                        get_media,
                        download_playlist,
                        upload_media,
                        send_media_by_id,
                        send_playlist,
                        add_downloaded_media,
                    })
                }
            ),
            provide(|
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(get_media): Inject<downloaded_media::GetRandomAudio>,
                Inject(send_playlist): Inject<send_media::id::SendAudioPlaylist<Messenger>>| {
                    Ok(audio::Random {
                        error_formatter,
                        get_media,
                        send_playlist,
                    })
                }
            ),
            provide(|
                Inject(cfg): Inject<Config>,
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(messenger): Inject<Messenger>,
                Inject(get_media): Inject<get_media::GetVideoByURL>,
                Inject(download_media): Inject<media::DownloadVideo>,
                Inject(upload_media): Inject<send_media::upload::SendVideo<Messenger>>,
                Inject(edit_media_by_id): Inject<send_media::id::EditVideo<Messenger>>,
                Inject(add_downloaded_media): Inject<downloaded_media::AddVideo>| {
                    Ok(chosen_inline::DownloadVideo {
                        cfg,
                        error_formatter,
                        messenger,
                        get_media,
                        download_media,
                        upload_media,
                        edit_media_by_id,
                        add_downloaded_media,
                    })
                }
            ),
            provide(|
                Inject(cfg): Inject<Config>,
                Inject(error_formatter): Inject<ErrorFormatter>,
                Inject(messenger): Inject<Messenger>,
                Inject(get_media): Inject<get_media::GetAudioByURL>,
                Inject(download_media): Inject<media::DownloadAudio>,
                Inject(upload_media): Inject<send_media::upload::SendAudio<Messenger>>,
                Inject(edit_media_by_id): Inject<send_media::id::EditAudio<Messenger>>,
                Inject(add_downloaded_media): Inject<downloaded_media::AddAudio>| {
                    Ok(chosen_inline::DownloadAudio {
                        cfg,
                        error_formatter,
                        messenger,
                        get_media,
                        download_media,
                        upload_media,
                        edit_media_by_id,
                        add_downloaded_media,
                    })
                }
            ),
        ],
        extend(cfg_registry, tg_messenger_registry, node_router_registry),
    }
}

pub(super) fn init(interactors_registry: Registry, database_registry: RegistryWithSync) -> Container {
    let registry = async_registry! {
        extend(interactors_registry, database_registry),
    };
    Container::new(registry)
}
