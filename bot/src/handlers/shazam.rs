use crate::{
    entities::ChatConfig,
    interactors::{shazam, Interactor as _},
    services::messenger::MessengerPort,
};

use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
    Extension,
};
use tracing::instrument;

#[instrument(skip_all)]
pub async fn shazam<Messenger>(
    message: Message,
    Extension(chat_cfg): Extension<ChatConfig>,
    Inject(interactor): Inject<shazam::Shazam<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    // The audio is either attached to this message (voice/audio with the command as a caption) or
    // in the message it replies to (the usual "reply /shazam to a voice" flow).
    let reply = message.reply_to_message();
    let source = reply.as_deref().unwrap_or(&message);
    let (file_id, file_size) = source
        .voice()
        .map(|voice| (voice.file_id.to_string(), voice.file_size))
        .or_else(|| source.audio().map(|audio| (audio.file_id.to_string(), audio.file_size)))
        .map_or((None, None), |(file_id, file_size)| (Some(file_id), file_size));

    interactor
        .execute(shazam::ShazamInput {
            chat_id: message.chat().id(),
            reply_to_message_id: message.message_id(),
            file_id,
            file_size,
            chat_cfg: Some(&chat_cfg),
        })
        .await?;
    Ok(EventReturn::Finish)
}
