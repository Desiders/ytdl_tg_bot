use crate::{
    interactors::{start, Interactor as _},
    services::messenger::MessengerPort,
};

use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
};
use tracing::instrument;

#[instrument(skip_all)]
pub async fn start<Messenger>(message: Message, Inject(interactor): Inject<start::Start<Messenger>>) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(start::StartInput {
            chat_id: message.chat().id(),
            reply_to_message_id: message.reply_to_message().as_ref().map(|message| message.message_id()),
        })
        .await?;
    Ok(EventReturn::Finish)
}
