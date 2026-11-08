use froodi::Inject;
use telers::{
    enums::ChatType,
    event::{telegram::HandlerResult, EventReturn},
    types::{CallbackQuery, Message},
    Extension,
};
use tracing::instrument;
use url::Url;

use crate::{
    entities::{ChatConfig, OwnChatConfig, Params},
    interactors::{quality, Interactor as _},
    services::messenger::MessengerPort,
    value_objects::quality::SelectQuality,
};

#[instrument(skip_all)]
pub async fn open<Messenger: MessengerPort>(
    message: Message,
    params: Params,
    url: Option<Extension<Url>>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Inject(interactor): Inject<quality::Open<Messenger>>,
) -> HandlerResult {
    interactor
        .execute(quality::OpenInput {
            chat_id: message.chat().id(),
            message_id: message.message_id(),
            is_private: ChatType::from(message.chat()) == ChatType::Private,
            url: url.as_ref().map(|url| &url.0),
            params: &params,
            chat_cfg: &chat_cfg,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn select<Messenger: MessengerPort>(
    query: CallbackQuery,
    data: SelectQuality,
    Extension(OwnChatConfig(chat_cfg)): Extension<OwnChatConfig>,
    Inject(interactor): Inject<quality::Select<Messenger>>,
) -> HandlerResult {
    let Some(message) = query.message.as_deref() else {
        return Ok(EventReturn::Finish);
    };
    let Some(chat_cfg) = chat_cfg else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(quality::SelectInput {
            chat_id: message.chat().id(),
            user_id: query.from.id,
            message_id: message.message_id(),
            callback_id: &query.id,
            data: &data,
            locale: chat_cfg.locale(),
        })
        .await?;
    Ok(EventReturn::Finish)
}
