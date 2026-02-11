use std::env;
use telers::{
    event::simple::HandlerResult,
    methods::SetMyCommands,
    types::{BotCommand, BotCommandScopeAllPrivateChats},
    Bot,
};
use tokio::fs;

async fn set_my_commands(bot: Bot) -> HandlerResult {
    let commands = [
        BotCommand::new("start", "Start the bot"),
        BotCommand::new("vd", "Download a video"),
        BotCommand::new("ad", "Download an audio"),
        BotCommand::new("rv", "Random a video"),
        BotCommand::new("ra", "Random an audio"),
    ];
    bot.send(SetMyCommands::new(commands).scope(BotCommandScopeAllPrivateChats {}))
        .await?;
    Ok(())
}

async fn remove_tmp_media_files() -> HandlerResult {
    let temp_dir = env::temp_dir();
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
pub async fn on_startup(bot: Bot) -> HandlerResult {
    set_my_commands(bot).await?;
    remove_tmp_media_files().await?;
    Ok(())
}
