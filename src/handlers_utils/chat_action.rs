use std::time::Duration;
use telers::{methods::SendChatAction, Bot};
use tracing::{event, Level};

const TIME_SLEEP_BETWEEN_SEND_ACTION_IN_MILLIS: u64 = 5000;

pub async fn upload_video_action_in_loop(bot: &Bot, chat_id: i64) {
    loop {
        if let Err(err) = bot.send(SendChatAction::new(chat_id, "upload_video")).await {
            event!(Level::ERROR, %err, "Error while sending upload action");

            break;
        }

        tokio::time::sleep(Duration::from_millis(TIME_SLEEP_BETWEEN_SEND_ACTION_IN_MILLIS)).await;
    }
}
