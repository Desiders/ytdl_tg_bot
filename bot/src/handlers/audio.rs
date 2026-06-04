use crate::{
    entities::{ChatConfig, OwnChatConfig, Params},
    interactors::{audio, enqueue_download, Interactor as _},
    services::messenger::MessengerPort,
    value_objects::MediaType,
};

use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
    Extension,
};
use tracing::instrument;
use url::Url;

#[instrument(skip_all, fields(%message_id = message.message_id(), %url = url.as_str(), ?params))]
pub async fn download<Messenger>(
    message: Message,
    params: Params,
    Extension(url): Extension<Url>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Inject(interactor): Inject<enqueue_download::EnqueueCommandDownload<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(enqueue_download::EnqueueCommandInput {
            media_type: MediaType::Audio,
            chat_id: message.chat().id(),
            message_id: message.message_id(),
            url: &url,
            params: &params,
            chat_cfg: &chat_cfg,
            link_is_visible: own_chat_cfg.as_ref().is_some_and(|chat_cfg| chat_cfg.link_is_visible),
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(%message_id = message.message_id()))]
pub async fn random<Messenger>(message: Message, params: Params, Inject(interactor): Inject<audio::Random<Messenger>>) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(audio::RandomInput {
            message_id: message.message_id(),
            chat_id: message.chat().id(),
            params: &params,
        })
        .await?;
    Ok(EventReturn::Finish)
}
