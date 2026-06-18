use crate::{
    entities::{ChatConfig, OwnChatConfig, Params},
    interactors::{enqueue_download, Interactor as _},
    services::messenger::MessengerPort,
    value_objects::MediaType,
};

use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::ChosenInlineResult,
    Extension,
};
use tracing::instrument;
use url::Url;

#[instrument(skip_all, fields(inline_message_id, url, ?params))]
pub async fn download_auto<Messenger>(
    params: Params,
    url_option: Option<Extension<Url>>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    ChosenInlineResult {
        inline_message_id,
        result_id,
        ..
    }: ChosenInlineResult,
    Inject(interactor): Inject<enqueue_download::EnqueueInlineDownload<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let url = url_option.as_ref().map(|Extension(url)| url);
    interactor
        .execute(enqueue_download::EnqueueInlineInput {
            media_type: MediaType::Video,
            inline_message_id,
            result_id: &result_id,
            url,
            params: &params,
            chat_cfg: &chat_cfg,
            link_is_visible: own_chat_cfg.as_ref().is_some_and(|chat_cfg| chat_cfg.link_is_visible),
            auto: true,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(inline_message_id, url, ?params))]
pub async fn download_video<Messenger>(
    params: Params,
    url_option: Option<Extension<Url>>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    ChosenInlineResult {
        inline_message_id,
        result_id,
        ..
    }: ChosenInlineResult,
    Inject(interactor): Inject<enqueue_download::EnqueueInlineDownload<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let url = url_option.as_ref().map(|Extension(url)| url);
    interactor
        .execute(enqueue_download::EnqueueInlineInput {
            media_type: MediaType::Video,
            inline_message_id,
            result_id: &result_id,
            url,
            params: &params,
            chat_cfg: &chat_cfg,
            link_is_visible: own_chat_cfg.as_ref().is_some_and(|chat_cfg| chat_cfg.link_is_visible),
            auto: false,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(inline_message_id, url, ?params))]
pub async fn download_audio<Messenger>(
    params: Params,
    url_option: Option<Extension<Url>>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    ChosenInlineResult {
        inline_message_id,
        result_id,
        ..
    }: ChosenInlineResult,
    Inject(interactor): Inject<enqueue_download::EnqueueInlineDownload<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let url = url_option.as_ref().map(|Extension(url)| url);
    interactor
        .execute(enqueue_download::EnqueueInlineInput {
            media_type: MediaType::Audio,
            inline_message_id,
            result_id: &result_id,
            url,
            params: &params,
            chat_cfg: &chat_cfg,
            link_is_visible: own_chat_cfg.as_ref().is_some_and(|chat_cfg| chat_cfg.link_is_visible),
            auto: false,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(inline_message_id, url, ?params))]
pub async fn download_photo<Messenger>(
    params: Params,
    url_option: Option<Extension<Url>>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    ChosenInlineResult {
        inline_message_id,
        result_id,
        ..
    }: ChosenInlineResult,
    Inject(interactor): Inject<enqueue_download::EnqueueInlineDownload<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let url = url_option.as_ref().map(|Extension(url)| url);
    interactor
        .execute(enqueue_download::EnqueueInlineInput {
            media_type: MediaType::Photo,
            inline_message_id,
            result_id: &result_id,
            url,
            params: &params,
            chat_cfg: &chat_cfg,
            link_is_visible: own_chat_cfg.as_ref().is_some_and(|chat_cfg| chat_cfg.link_is_visible),
            auto: false,
        })
        .await?;
    Ok(EventReturn::Finish)
}
