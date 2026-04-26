use crate::{
    database::TxManager,
    entities::ChatConfig,
    interactors::{lang, Interactor as _},
    services::messenger::MessengerPort,
};

use froodi::{Inject, InjectTransient};
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
    Extension,
};
use tracing::instrument;

#[instrument(skip_all)]
pub async fn lang<Messenger>(
    message: Message,
    Extension(chat_cfg): Extension<ChatConfig>,
    Inject(interactor): Inject<lang::Lang<Messenger>>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let argument = extract_argument(message.text());
    interactor
        .execute(lang::LangInput {
            reply_to_message_id: message.reply_to_message().as_ref().map(|message| message.message_id()),
            chat_cfg: &chat_cfg,
            argument: argument.as_deref(),
            tx_manager: &mut tx_manager,
        })
        .await?;
    Ok(EventReturn::Finish)
}

fn extract_argument(text: Option<&str>) -> Option<String> {
    let raw = text?.trim();
    let mut parts = raw.splitn(2, char::is_whitespace);
    let _command = parts.next()?;
    parts.next().map(|rest| rest.trim().to_owned()).filter(|s| !s.is_empty())
}
