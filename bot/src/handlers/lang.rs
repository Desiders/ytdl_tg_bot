use crate::{
    entities::ChatConfig,
    interactors::{lang, Interactor as _},
    services::messenger::MessengerPort,
};

use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    filters::CommandObject,
    types::Message,
    Extension,
};
use tracing::instrument;

#[instrument(skip_all)]
pub async fn lang<Messenger>(
    message: Message,
    command: CommandObject,
    Extension(chat_cfg): Extension<ChatConfig>,
    Inject(interactor): Inject<lang::Lang<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let argument = command.args.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(" ");
    interactor
        .execute(lang::LangInput {
            reply_to_message_id: message.reply_to_message().as_ref().map(|&message| message.message_id()),
            chat_cfg: &chat_cfg,
            argument: (!argument.is_empty()).then_some(argument.as_str()),
        })
        .await?;
    Ok(EventReturn::Finish)
}
