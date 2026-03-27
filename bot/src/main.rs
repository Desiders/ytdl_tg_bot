mod config;
mod database;
mod di_container;
mod entities;
mod errors;
mod filters;
mod handlers;
mod handlers_utils;
mod interactors;
mod middlewares;
mod services;
mod utils;
mod value_objects;

use froodi::telers::setup_async_default;
use services::node_router::NodeRouter;
use std::borrow::Cow;
use telers::{
    client::{
        telegram::{APIServer, BareFilesPathWrapper},
        Reqwest,
    },
    enums::{ChatType::Private, MessageType::Text},
    event::{simple::Handler as SimpleHandler, telegram::Handler},
    filters::{ChatType, Command, Filter as _, MessageType},
    Bot, Dispatcher, Router,
};
use tracing::{error, info};
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};

use crate::{
    filters::{
        is_audio_inline_result, is_exclude_domain, is_via_bot, is_video_inline_result, random_cmd_is_enabled,
        text_contains_host_with_reply, text_contains_url, text_contains_url_with_reply, text_empty, url_is_blacklisted,
        url_is_skippable_by_param,
    },
    handlers::{audio, chosen_inline, inline_query, start, stats, video},
    middlewares::{CreateChatMiddleware, ReactionMiddleware, RemoveTrackingParamsMiddleware, ReplaceDomainsMiddleware},
    utils::{on_shutdown, on_startup},
};

#[tokio::main(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)]
async fn main() {
    let config_path = config::get_path();
    let config = config::parse_from_fs(&*config_path).unwrap();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::builder().parse_lossy(config.logging.dirs.as_ref()))
        .init();

    let download_node_addresses: Vec<_> = config.download.nodes.iter().map(|node| node.address.as_ref()).collect();
    info!(
        config_path = %config_path,
        log_filter = %config.logging.dirs,
        yt_toolkit_url = %config.yt_toolkit.url,
        telegram_bot_api_url = %config.telegram_bot_api.url,
        download_node_count = config.download.nodes.len(),
        download_node_addresses = ?download_node_addresses,
        capabilities_refresh_interval = config.download.capabilities_refresh_interval,
        max_file_size = config.yt_dlp.max_file_size,
        "Loaded bot config"
    );

    let base_url = format!("{}/bot{{token}}/{{method_name}}", config.telegram_bot_api.url);
    let files_url = format!("{}/file{{token}}/{{path}}", config.telegram_bot_api.url);

    let bot = Bot::with_client(
        config.bot.token.clone(),
        Reqwest::default().with_api_server(Cow::Owned(APIServer::new(&base_url, &files_url, true, BareFilesPathWrapper))),
    );

    let container = di_container::init(bot.clone(), config);
    let node_router = container.get::<NodeRouter>().await.unwrap();
    let cfg = container.get::<config::Config>().await.unwrap();

    let download_router = Router::new("download")
        .on_message(|observer| {
            observer
                .register_inner_middleware(RemoveTrackingParamsMiddleware)
                .register_inner_middleware(ReplaceDomainsMiddleware)
                .register_inner_middleware(ReactionMiddleware)
                .register(
                    Handler::new(video::download)
                        .filter(MessageType::one(Text))
                        .filter(Command::many(["vd", "video", "video_download"]))
                        .filter(text_contains_url_with_reply),
                )
                .register(
                    Handler::new(audio::download)
                        .filter(MessageType::one(Text))
                        .filter(Command::many(["ad", "audio", "audio_download"]))
                        .filter(text_contains_url_with_reply),
                )
                .register(
                    Handler::new(video::random)
                        .filter(MessageType::one(Text))
                        .filter(Command::many(["rv", "random_video"]))
                        .filter(random_cmd_is_enabled),
                )
                .register(
                    Handler::new(audio::random)
                        .filter(MessageType::one(Text))
                        .filter(Command::many(["ra", "random_audio"]))
                        .filter(random_cmd_is_enabled),
                )
                .register(
                    Handler::new(handlers::config::add_exclude_domain)
                        .filter(MessageType::one(Text))
                        .filter(Command::many(["add_ed", "add_exclude_domain"]))
                        .filter(text_contains_host_with_reply),
                )
                .register(
                    Handler::new(handlers::config::remove_exclude_domain)
                        .filter(MessageType::one(Text))
                        .filter(Command::many(["rm_ed", "remove_ed", "rm_exclude_domain", "remove_exclude_domain"]))
                        .filter(text_contains_host_with_reply),
                )
                .register(
                    Handler::new(handlers::config::change_link_visibility)
                        .filter(MessageType::one(Text))
                        .filter(Command::one("change_link_visibility")),
                )
                .register(
                    Handler::new(video::download)
                        .filter(ChatType::one(Private))
                        .filter(text_contains_url_with_reply)
                        .filter(is_via_bot.invert())
                        .filter(is_exclude_domain.invert()),
                )
                .register(
                    Handler::new(video::download_quiet)
                        .filter(text_contains_url)
                        .filter(url_is_blacklisted.invert())
                        .filter(url_is_skippable_by_param.invert())
                        .filter(is_via_bot.invert())
                        .filter(is_exclude_domain.invert()),
                )
        })
        .on_inline_query(|observer| {
            observer
                .register_inner_middleware(RemoveTrackingParamsMiddleware)
                .register(Handler::new(inline_query::select_by_url).filter(text_contains_url))
                .register(Handler::new(inline_query::select_by_text).filter(text_empty.invert()))
        })
        .on_chosen_inline_result(|observer| {
            observer
                .register_inner_middleware(RemoveTrackingParamsMiddleware)
                .register_inner_middleware(ReplaceDomainsMiddleware)
                .register(
                    Handler::new(chosen_inline::download_video)
                        .filter(is_video_inline_result)
                        .filter(text_contains_url.or(text_empty.invert())),
                )
                .register(
                    Handler::new(chosen_inline::download_audio)
                        .filter(is_audio_inline_result)
                        .filter(text_contains_url.or(text_empty.invert())),
                )
        });

    let router = setup_async_default(Router::new("main"), container.clone())
        .on_update(|observer| observer.register_outer_middleware(CreateChatMiddleware))
        .on_message(|observer| {
            observer
                .register(Handler::new(start).filter(Command::many(["start", "help"])))
                .register(Handler::new(stats).filter(Command::one("stats")))
        })
        .on_startup(|observer| observer.register(SimpleHandler::new(on_startup, (bot.clone(), node_router.clone(), cfg.clone()))))
        .on_shutdown(|observer| observer.register(SimpleHandler::new(on_shutdown, ())))
        .include(download_router);

    let dispatcher = Dispatcher::builder()
        .allowed_updates(router.resolve_used_update_types())
        .main_router(router.configure_default())
        .bot(bot)
        .build();

    match dispatcher.run_polling().await {
        Ok(()) => {
            info!("Bot stopped");
        }
        Err(err) => {
            error!(error = %err, "Bot stopped");
        }
    }

    container.close().await;
}
