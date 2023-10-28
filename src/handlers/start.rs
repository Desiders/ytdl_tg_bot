use telers::{
    event::{telegram::HandlerResult, EventReturn},
    methods::SendMessage,
    types::Message,
    Bot,
};

pub async fn start(bot: Bot, message: Message) -> HandlerResult {
    let text = format!(
        "Hello, {first_name}!\n\nSend me a link to a YouTube video and I'll send you the video file!",
        first_name = message.from.as_ref().map_or("Anonymous", |user| user.first_name.as_ref()),
    );

    bot.send(&SendMessage::new(message.chat_id(), text)).await?;

    Ok(EventReturn::Finish)
}
