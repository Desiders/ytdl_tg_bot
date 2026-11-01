use super::menu::message_access_input;

use crate::{
    entities::OwnChatConfig,
    interactors::{lang, Interactor as _},
    services::messenger::{EphemeralMessage, MessengerPort},
};

use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    filters::CommandObject,
    types::{Chat, Message},
    Extension,
};
use tracing::instrument;

#[instrument(skip_all)]
pub async fn lang<Messenger>(
    message: Message,
    command: CommandObject,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Inject(interactor): Inject<lang::Lang<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    let argument = command.args.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(" ");
    let ephemeral = if matches!(message.chat(), Chat::Group(_) | Chat::Supergroup(_)) {
        message.from().map(|user| EphemeralMessage {
            receiver_user_id: user.id,
            reply_to_ephemeral_message_id: message.ephemeral_message_id(),
        })
    } else {
        None
    };
    interactor
        .execute(lang::LangInput {
            access: message_access_input(&message),
            chat_id: message.chat().id(),
            reply_to_message_id: message.reply_to_message().map(Message::message_id).filter(|&id| id > 0),
            ephemeral,
            chat_cfg,
            argument: (!argument.is_empty()).then_some(argument.as_str()),
        })
        .await?;
    Ok(EventReturn::Finish)
}
