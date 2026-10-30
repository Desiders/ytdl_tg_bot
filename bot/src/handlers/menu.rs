use crate::{
    entities::{ChatConfig, ChatConfigExcludeDomains, OwnChatConfig},
    filters::get_host_from_text,
    interactors::{menu, Interactor as _},
    services::messenger::{EphemeralMenu, MenuMessage, MessengerPort},
    value_objects::menu::{DeleteDomain, OpenScreen, Screen, SetLanguage, SetLinkVisibility},
};

use froodi::Inject;
use telers::{
    errors::HandlerError,
    event::{telegram::HandlerResult, EventReturn},
    types::{CallbackQuery, Chat, Message},
    Extension,
};
use tracing::instrument;

#[instrument(skip_all)]
pub async fn open_main<Messenger>(
    message: Message,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(menu::OpenMenuInput {
            target: send_target(&message, true)?,
            screen: Screen::Main,
            notice: None,
            chat_cfg: own_chat_cfg.as_ref().unwrap_or(&chat_cfg),
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn open_help<Messenger>(
    message: Message,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(menu::OpenMenuInput {
            target: send_target(&message, false)?,
            screen: Screen::Help,
            notice: None,
            chat_cfg: own_chat_cfg.as_ref().unwrap_or(&chat_cfg),
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn open_stats<Messenger>(
    message: Message,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(menu::OpenMenuInput {
            target: send_target(&message, false)?,
            screen: Screen::Stats,
            notice: None,
            chat_cfg: own_chat_cfg.as_ref().unwrap_or(&chat_cfg),
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn open_language<Messenger>(
    message: Message,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(menu::OpenMenuInput {
            target: send_target(&message, false)?,
            screen: Screen::Language,
            notice: None,
            chat_cfg: own_chat_cfg.as_ref().unwrap_or(&chat_cfg),
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn add_domain<Messenger>(
    message: Message,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::AddMenuDomain<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let host = message.text().and_then(get_host_from_text).map(|host| host.to_string());
    interactor
        .execute(menu::AddMenuDomainInput {
            target: domain_target(&message, &fsm).await?,
            host: host.as_deref(),
            chat_cfg: own_chat_cfg.as_ref().unwrap_or(&chat_cfg),
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn open_screen<Messenger>(
    query: CallbackQuery,
    data: OpenScreen,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(target) = edit_target(&query) else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::OpenMenuInput {
            target,
            screen: data.screen,
            notice: None,
            chat_cfg: own_chat_cfg.as_ref().unwrap_or(&chat_cfg),
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn set_language<Messenger>(
    query: CallbackQuery,
    data: SetLanguage,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::SetMenuLanguage<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(target) = edit_target(&query) else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::SetMenuLanguageInput {
            target,
            locale: data.locale,
            chat_cfg: own_chat_cfg.as_ref().unwrap_or(&chat_cfg),
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn set_link_visibility<Messenger>(
    query: CallbackQuery,
    data: SetLinkVisibility,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::SetMenuLinkVisibility<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(target) = edit_target(&query) else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::SetMenuLinkVisibilityInput {
            target,
            link_is_visible: data.visible,
            chat_cfg: own_chat_cfg.as_ref().unwrap_or(&chat_cfg),
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn remove_domain<Messenger>(
    query: CallbackQuery,
    data: DeleteDomain,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::RemoveMenuDomain<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(target) = edit_target(&query) else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::RemoveMenuDomainInput {
            target,
            domain: data.domain,
            chat_cfg: own_chat_cfg.as_ref().unwrap_or(&chat_cfg),
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

fn send_target(message: &Message, remove_command: bool) -> Result<menu::MenuTarget<'static>, HandlerError> {
    let is_group = matches!(message.chat(), Chat::Group(_) | Chat::Supergroup(_));
    let ephemeral = if is_group {
        let user = message.from().ok_or_else(|| HandlerError::from_display("Missing menu owner"))?;
        Some(EphemeralMenu {
            receiver_user_id: user.id,
            reply_to_ephemeral_message_id: message.ephemeral_message_id(),
        })
    } else {
        None
    };
    Ok(menu::MenuTarget::Send {
        chat_id: message.chat().id(),
        ephemeral,
        command_message_id: (is_group && remove_command && message.message_id() > 0).then_some(message.message_id()),
    })
}

fn edit_target(query: &CallbackQuery) -> Option<menu::MenuTarget<'_>> {
    let message = query.message.as_deref()?;
    let target = if let Some(ephemeral_message_id) = message.ephemeral_message_id() {
        let receiver = message.receiver_user()?;
        if receiver.id != query.from.id {
            return None;
        }
        MenuMessage::Ephemeral {
            chat_id: message.chat().id(),
            receiver_user_id: receiver.id,
            ephemeral_message_id,
        }
    } else if matches!(message.chat(), Chat::Private(_)) {
        MenuMessage::Chat {
            chat_id: message.chat().id(),
            message_id: message.message_id(),
        }
    } else {
        return None;
    };
    Some(menu::MenuTarget::Edit {
        message: target,
        callback_id: Some(&query.id),
    })
}

async fn domain_target(message: &Message, fsm: &menu::Fsm) -> Result<menu::MenuTarget<'static>, HandlerError> {
    if !matches!(message.chat(), Chat::Private(_)) {
        let ephemeral_message_id = fsm
            .get_value::<_, i64>(menu::EPHEMERAL_MESSAGE_ID)
            .await
            .map_err(HandlerError::new)?
            .ok_or_else(|| HandlerError::from_display("Missing menu message"))?;
        let user = message.from().ok_or_else(|| HandlerError::from_display("Missing menu owner"))?;
        return Ok(menu::MenuTarget::Edit {
            message: MenuMessage::Ephemeral {
                chat_id: message.chat().id(),
                receiver_user_id: user.id,
                ephemeral_message_id,
            },
            callback_id: None,
        });
    }
    send_target(message, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use telers::{
        fsm::{MemoryStorage, StorageKey},
        types::{ChatGroup, ChatPrivate, MessageText, User},
    };

    #[test]
    fn group_menus_use_ephemeral_ids_and_only_delete_visible_commands() {
        let user = User::new(2, false, "Owner");
        let command = Message::Text(MessageText::new(7, 1, ChatGroup::new(-1), "/start").from(user.clone()));
        assert!(matches!(
            send_target(&command, true).unwrap(),
            menu::MenuTarget::Send {
                chat_id: -1,
                command_message_id: Some(7),
                ephemeral: Some(EphemeralMenu {
                    receiver_user_id: 2,
                    reply_to_ephemeral_message_id: None,
                }),
            }
        ));

        let ephemeral = MessageText::new(0, 1, ChatGroup::new(-1), "/start")
            .from(user.clone())
            .receiver_user(user.clone())
            .ephemeral_message_id(9);
        assert!(matches!(
            send_target(&Message::Text(ephemeral.clone()), true).unwrap(),
            menu::MenuTarget::Send {
                command_message_id: None,
                ephemeral: Some(EphemeralMenu {
                    reply_to_ephemeral_message_id: Some(9),
                    ..
                }),
                ..
            }
        ));
        let query = CallbackQuery::new("synthetic", user.clone(), "synthetic").message(Message::Text(ephemeral));
        assert!(matches!(
            edit_target(&query),
            Some(menu::MenuTarget::Edit {
                message: MenuMessage::Ephemeral {
                    chat_id: -1,
                    receiver_user_id: 2,
                    ephemeral_message_id: 9,
                },
                callback_id: Some("synthetic"),
            })
        ));
        assert!(edit_target(&query.from(User::new(3, false, "Other"))).is_none());

        let private = Message::Text(MessageText::new(7, 1, ChatPrivate::new(2), "/start").from(user));
        assert!(matches!(
            send_target(&private, true).unwrap(),
            menu::MenuTarget::Send {
                ephemeral: None,
                command_message_id: None,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn typed_group_domains_edit_the_prompt_and_require_its_id() {
        let fsm = menu::Fsm::new(MemoryStorage::new(), StorageKey::new(1, -1, 2, None, None));
        let message = Message::Text(MessageText::new(7, 1, ChatGroup::new(-1), "example.com").from(User::new(2, false, "Owner")));
        assert!(domain_target(&message, &fsm).await.is_err());
        fsm.set_value(menu::EPHEMERAL_MESSAGE_ID, 9_i64).await.unwrap();
        assert!(matches!(
            domain_target(&message, &fsm).await.unwrap(),
            menu::MenuTarget::Edit {
                message: MenuMessage::Ephemeral {
                    chat_id: -1,
                    receiver_user_id: 2,
                    ephemeral_message_id: 9,
                },
                callback_id: None,
            }
        ));
    }
}
