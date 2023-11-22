use crate::config::{Bot as BotConfig, PhantomVideo as PhantomVideoConfig, PhantomVideoId};

use telers::{
    errors::SessionErrorKind,
    methods::{DeleteMessage, SendVideo},
    types::InputFile,
    Bot,
};
use tracing::{event, Level};

pub async fn get_phantom_video_id(
    bot: Bot,
    bot_config: BotConfig,
    phantom_video_config: PhantomVideoConfig,
) -> Result<PhantomVideoId, SessionErrorKind> {
    match phantom_video_config {
        PhantomVideoConfig::Id(id) => {
            event!(Level::DEBUG, ?id, "Got phantom video id from config");

            Ok(id)
        }
        PhantomVideoConfig::Path(path) => {
            event!(Level::DEBUG, ?path, "Got phantom video path from config");

            let phantom_file = InputFile::fs(path);

            event!(Level::DEBUG, ?phantom_file, "Sending phantom video");

            let message = bot
                .send(SendVideo::new(bot_config.receiver_video_chat_id, phantom_file).disable_notification(true))
                .await?;

            tokio::spawn(async move {
                bot.send(DeleteMessage::new(bot_config.receiver_video_chat_id, message.message_id))
                    .await
            });

            // `unwrap` is safe because we checked that `message.video` is `Some` in `SendVideo` method
            Ok(PhantomVideoId(message.video.unwrap().file_id.into_string()))
        }
    }
}
