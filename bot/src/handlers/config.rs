use super::menu::message_access_input;

use crate::entities::{ChatConfig, ChatConfigExcludeDomains, OwnChatConfig};
use crate::interactors::{config, Interactor as _};
use crate::services::messenger::{MessengerPort, SendTextRequest};

use froodi::Inject;
use rust_i18n::t;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
    Extension,
};
use tracing::instrument;
use url::Host;

pub async fn change_link_visibility<Messenger>(
    message: Message,
    Extension(chat_cfg): Extension<ChatConfig>,
    Inject(interactor): Inject<config::ChangeLinkVisibility<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(config::ChangeLinkVisibilityInput {
            reply_to_message_id: message.reply_to_message().as_ref().map(|message| message.message_id()),
            chat_cfg: &chat_cfg,
        })
        .await?;
    Ok(EventReturn::Finish)
}

pub async fn change_link_visibility_private_only<Messenger>(
    message: Message,
    Extension(chat_cfg): Extension<ChatConfig>,
    Inject(messenger): Inject<Messenger>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let text = t!("link_visibility.private_only", locale = chat_cfg.locale().as_str()).into_owned();
    let _ = messenger
        .send_text(SendTextRequest {
            chat_id: message.chat().id(),
            text: &text,
            reply_to_message_id: Some(message.message_id()),
            ephemeral: None,
            format: None,
            disable_link_preview: true,
        })
        .await;

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(%message_id = message.message_id(), %host))]
pub async fn add_exclude_domain<Messenger>(
    message: Message,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(chat_cfg_domains): Extension<ChatConfigExcludeDomains>,
    Extension(host): Extension<Host>,
    Inject(interactor): Inject<config::AddExcludeDomain<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    let host = host.to_string();
    interactor
        .execute(config::AddExcludeDomainInput {
            access: message_access_input(&message),
            chat_id: message.chat().id(),
            reply_to_message_id: message.reply_to_message().as_ref().map(|message| message.message_id()),
            host: &host,
            exclude_domains: &chat_cfg_domains,
            chat_cfg,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(%message_id = message.message_id(), %host))]
pub async fn remove_exclude_domain<Messenger>(
    message: Message,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(chat_cfg_domains): Extension<ChatConfigExcludeDomains>,
    Extension(host): Extension<Host>,
    Inject(interactor): Inject<config::RemoveExcludeDomain<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    let host = host.to_string();
    interactor
        .execute(config::RemoveExcludeDomainInput {
            access: message_access_input(&message),
            chat_id: message.chat().id(),
            reply_to_message_id: message.reply_to_message().as_ref().map(|message| message.message_id()),
            host: &host,
            exclude_domains: &chat_cfg_domains,
            chat_cfg,
        })
        .await?;
    Ok(EventReturn::Finish)
}
