use crate::config::{Bot as BotConfig, PhantomAudio as PhantomAudioConfig, PhantomAudioId};

use telers::{
    errors::SessionErrorKind,
    methods::{DeleteMessage, SendAudio},
    types::InputFile,
    Bot,
};
use tracing::{event, Level};

pub async fn get_phantom_audio_id(
    bot: Bot,
    bot_config: BotConfig,
    phantom_audio_config: PhantomAudioConfig,
) -> Result<PhantomAudioId, SessionErrorKind> {
    match phantom_audio_config {
        PhantomAudioConfig::Id(id) => {
            event!(Level::DEBUG, ?id, "Got phantom audio id from config");

            Ok(id)
        }
        PhantomAudioConfig::Path(path) => {
            event!(Level::DEBUG, ?path, "Got phantom audio path from config");

            let phantom_file = InputFile::fs(path);

            event!(Level::DEBUG, ?phantom_file, "Sending phantom video");

            let message = bot
                .send(
                    SendAudio::new(bot_config.receiver_video_chat_id, phantom_file)
                        .title("Audio of the video")
                        .performer("Click to download audio")
                        .duration(0)
                        .disable_notification(true),
                )
                .await?;

            tokio::spawn({
                let message_id = message.id();

                async move { bot.send(DeleteMessage::new(bot_config.receiver_video_chat_id, message_id)).await }
            });

            // `unwrap` is safe because we checked that `message.audio` is `Some` in `SendAudio` method
            Ok(PhantomAudioId(message.audio().unwrap().file_id.clone().into_string()))
        }
    }
}
