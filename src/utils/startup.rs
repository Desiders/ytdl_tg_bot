use telers::{
    event::simple::HandlerResult,
    methods::SetMyCommands,
    types::{BotCommand, BotCommandScopeAllPrivateChats},
    Bot,
};

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

#[allow(clippy::module_name_repetitions)]
pub async fn on_startup(bot: Bot) -> HandlerResult {
    set_my_commands(bot).await
}
