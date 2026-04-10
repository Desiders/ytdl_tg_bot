use crate::{
    database::TxManager,
    interactors::{stats, Interactor as _},
    services::messenger::MessengerPort,
};

use froodi::{Inject, InjectTransient};
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
};
use tracing::instrument;

#[instrument(skip_all)]
pub async fn stats<Messenger>(
    message: Message,
    Inject(interactor): Inject<stats::Stats<Messenger>>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(stats::StatsInput {
            chat_id: message.chat().id(),
            reply_to_message_id: message.reply_to_message().as_ref().map(|message| message.message_id()),
            tx_manager: &mut tx_manager,
        })
        .await?;
    Ok(EventReturn::Finish)
}
