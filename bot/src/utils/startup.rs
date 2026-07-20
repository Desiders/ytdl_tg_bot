use rust_i18n::t;
use std::{env, sync::Arc, time::Duration};
use telers::{
    event::simple::HandlerResult,
    methods::SetMyCommands,
    types::{BotCommand, BotCommandScopeAllPrivateChats},
    Bot,
};
use tokio::fs;
use tracing::{debug, info};

use crate::{config::Config, locale::Locale, services::node_router::NodeRouter};

const COMMAND_KEYS: &[&str] = &[
    "start",
    "vd",
    "ad",
    "rv",
    "ra",
    "add_ed",
    "rm_ed",
    "change_link_visibility",
    "shazam",
    "stats",
    "lang",
];

fn build_commands(locale: &str) -> Vec<BotCommand> {
    COMMAND_KEYS
        .iter()
        .map(|key| BotCommand::new(*key, t!(format!("bot_commands.{key}"), locale = locale).as_ref()))
        .collect()
}

async fn set_my_commands(bot: Bot) -> HandlerResult {
    bot.send(SetMyCommands::new(build_commands(Locale::En.as_str())).scope(BotCommandScopeAllPrivateChats {}))
        .await?;
    bot.send(
        SetMyCommands::new(build_commands(Locale::Ru.as_str()))
            .scope(BotCommandScopeAllPrivateChats {})
            .language_code(Locale::Ru.as_str()),
    )
    .await?;
    bot.send(
        SetMyCommands::new(build_commands(Locale::Uk.as_str()))
            .scope(BotCommandScopeAllPrivateChats {})
            .language_code(Locale::Uk.as_str()),
    )
    .await?;
    Ok(())
}

async fn remove_tmp_media_files() -> HandlerResult {
    let temp_dir = env::temp_dir();
    debug!(temp_dir = %temp_dir.display(), "Cleaning temporary media directories");
    let mut entries = fs::read_dir(&temp_dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let file_type = entry.file_type().await?;
        if file_type.is_dir() {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();

            if file_name.starts_with("ytdl-tg-bot") {
                let path = entry.path();
                fs::remove_dir_all(&path).await?;
            }
        }
    }

    Ok(())
}

#[allow(clippy::module_name_repetitions)]
pub async fn on_startup(bot: Bot, node_router: Arc<NodeRouter>, cfg: Arc<Config>) -> HandlerResult {
    set_my_commands(bot).await?;
    remove_tmp_media_files().await?;

    info!("Running initial downloader node status refresh");
    node_router.refresh_status().await;

    if cfg.download.capabilities_refresh_interval > 0 {
        info!("Running initial downloader node capabilities refresh");
        node_router.refresh_capabilities().await;
    }

    {
        let router = node_router.clone();
        info!(interval_sec = %5, "Starting node status refresh task");
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                router.refresh_status().await;
            }
        });
    }

    if cfg.download.capabilities_refresh_interval > 0 {
        let router = node_router.clone();
        let cfg = cfg.clone();
        info!(
            interval_sec = %cfg.download.capabilities_refresh_interval,
            "Starting node capabilities refresh task"
        );
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(cfg.download.capabilities_refresh_interval));
            loop {
                interval.tick().await;
                router.refresh_capabilities().await;
            }
        });
    }

    info!("Startup sequence completed");
    Ok(())
}
