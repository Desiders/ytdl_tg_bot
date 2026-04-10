use crate::{
    database::TxManager,
    entities::{ChatConfig, Params},
    interactors::{video, Interactor as _},
    services::messenger::MessengerPort,
};

use froodi::{Inject, InjectTransient};
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
    Inject(interactor): Inject<video::Download<Messenger>>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(video::DownloadInput {
            message_id: message.message_id(),
            chat_id: message.chat().id(),
            params: &params,
            url: &url,
            chat_cfg: &chat_cfg,
            tx_manager: &mut tx_manager,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(%message_id = message.message_id(), %url = url.as_str(), ?params))]
pub async fn download_quiet<Messenger>(
    message: Message,
    params: Params,
    Extension(url): Extension<Url>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Inject(interactor): Inject<video::DownloadQuiet<Messenger>>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(video::DownloadQuietInput {
            message_id: message.message_id(),
            chat_id: message.chat().id(),
            params: &params,
            url: &url,
            chat_cfg: &chat_cfg,
            tx_manager: &mut tx_manager,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(%message_id = message.message_id()))]
pub async fn random<Messenger>(
    message: Message,
    params: Params,
    Inject(interactor): Inject<video::Random<Messenger>>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(video::RandomInput {
            message_id: message.message_id(),
            chat_id: message.chat().id(),
            params: &params,
            tx_manager: &mut tx_manager,
        })
        .await?;
    Ok(EventReturn::Finish)
}
