use crate::{
    entities::{ChatConfig, ChatConfigExcludeDomains, OwnChatConfig},
    filters::get_host_from_text,
    interactors::{
        menu,
        menu_access::{CheckMenuAccess, CheckMenuAccessInput},
        Interactor as _,
    },
    services::messenger::{EphemeralMessage, MenuMessage, MessengerPort},
    value_objects::menu::{DeleteDomain, OpenScreen, Screen, SetLanguage, SetLinkVisibility},
};

use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::{CallbackQuery, Chat, Message},
    Extension,
};
use tracing::instrument;

#[instrument(skip_all)]
pub async fn open_main<Messenger>(
    message: Message,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(access): Inject<CheckMenuAccess<Messenger>>,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    if !access.execute(message_access_input(&message, chat_cfg, &fsm, false)).await? {
        return Ok(EventReturn::Finish);
    }

    interactor
        .execute(menu::OpenMenuInput {
            target: send_target(&message),
            screen: Screen::Main,
            notice: None,
            chat_cfg,
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn open_help<Messenger>(
    message: Message,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(access): Inject<CheckMenuAccess<Messenger>>,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    if !access.execute(message_access_input(&message, chat_cfg, &fsm, false)).await? {
        return Ok(EventReturn::Finish);
    }

    interactor
        .execute(menu::OpenMenuInput {
            target: send_target(&message),
            screen: Screen::Help,
            notice: None,
            chat_cfg,
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn open_stats<Messenger>(
    message: Message,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(access): Inject<CheckMenuAccess<Messenger>>,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    if !access.execute(message_access_input(&message, chat_cfg, &fsm, false)).await? {
        return Ok(EventReturn::Finish);
    }

    interactor
        .execute(menu::OpenMenuInput {
            target: send_target(&message),
            screen: Screen::Stats,
            notice: None,
            chat_cfg,
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn open_language<Messenger>(
    message: Message,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(access): Inject<CheckMenuAccess<Messenger>>,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    if !access.execute(message_access_input(&message, chat_cfg, &fsm, false)).await? {
        return Ok(EventReturn::Finish);
    }

    interactor
        .execute(menu::OpenMenuInput {
            target: send_target(&message),
            screen: Screen::Language,
            notice: None,
            chat_cfg,
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all)]
pub async fn add_domain<Messenger>(
    message: Message,
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(access): Inject<CheckMenuAccess<Messenger>>,
    Inject(interactor): Inject<menu::AddMenuDomain<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    if !access.execute(message_access_input(&message, chat_cfg, &fsm, true)).await? {
        return Ok(EventReturn::Finish);
    }

    let host = message.text().and_then(get_host_from_text).map(|host| host.to_string());
    interactor
        .execute(menu::AddMenuDomainInput {
            target: send_target(&message),
            host: host.as_deref(),
            chat_cfg,
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
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(access): Inject<CheckMenuAccess<Messenger>>,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    let Some(input) = callback_access_input(
        &query,
        chat_cfg,
        &fsm,
        matches!(
            data.screen,
            Screen::Domains | Screen::DomainAdd | Screen::Domain { .. } | Screen::DomainDelete { .. }
        ),
    ) else {
        return Ok(EventReturn::Finish);
    };
    if !access.execute(input).await? {
        return Ok(EventReturn::Finish);
    }

    let Some(target) = edit_target(&query) else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::OpenMenuInput {
            target,
            screen: data.screen,
            notice: None,
            chat_cfg,
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
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(access): Inject<CheckMenuAccess<Messenger>>,
    Inject(interactor): Inject<menu::SetMenuLanguage<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    let Some(input) = callback_access_input(&query, chat_cfg, &fsm, false) else {
        return Ok(EventReturn::Finish);
    };
    if !access.execute(input).await? {
        return Ok(EventReturn::Finish);
    }

    let Some(target) = edit_target(&query) else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::SetMenuLanguageInput {
            target,
            locale: data.locale,
            chat_cfg,
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
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(access): Inject<CheckMenuAccess<Messenger>>,
    Inject(interactor): Inject<menu::SetMenuLinkVisibility<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    let Some(input) = callback_access_input(&query, chat_cfg, &fsm, false) else {
        return Ok(EventReturn::Finish);
    };
    if !access.execute(input).await? {
        return Ok(EventReturn::Finish);
    }

    let Some(target) = edit_target(&query) else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::SetMenuLinkVisibilityInput {
            target,
            link_is_visible: data.visible,
            chat_cfg,
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
    Extension(OwnChatConfig(own_chat_cfg)): Extension<OwnChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(access): Inject<CheckMenuAccess<Messenger>>,
    Inject(interactor): Inject<menu::RemoveMenuDomain<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(chat_cfg) = own_chat_cfg.as_ref() else {
        return Ok(EventReturn::Finish);
    };

    let Some(input) = callback_access_input(&query, chat_cfg, &fsm, true) else {
        return Ok(EventReturn::Finish);
    };
    if !access.execute(input).await? {
        return Ok(EventReturn::Finish);
    }

    let Some(target) = edit_target(&query) else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::RemoveMenuDomainInput {
            target,
            domain: data.domain,
            chat_cfg,
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}

fn send_target(message: &Message) -> menu::MenuTarget<'static> {
    let is_group = matches!(message.chat(), Chat::Group(_) | Chat::Supergroup(_));
    let ephemeral = is_group.then(|| EphemeralMessage {
        receiver_user_id: message.from().unwrap().id,
        reply_to_ephemeral_message_id: message.ephemeral_message_id(),
    });
    menu::MenuTarget::Send {
        chat_id: message.chat().id(),
        ephemeral,
        command_message_id: (is_group && message.message_id() > 0).then_some(message.message_id()),
    }
}

fn edit_target(query: &CallbackQuery) -> Option<menu::MenuTarget<'_>> {
    let message = query.message.as_deref()?;
    let target = if let Some(ephemeral_message_id) = message.ephemeral_message_id() {
        let receiver = message.receiver_user()?;
        MenuMessage::EphemeralMessage {
            chat_id: message.chat().id(),
            receiver_user_id: receiver.id,
            ephemeral_message_id,
        }
    } else if matches!(message.chat(), Chat::Private(_)) {
        MenuMessage::ChatMessage {
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

pub(super) fn message_access_input<'a>(
    message: &Message,
    chat_cfg: &'a ChatConfig,
    fsm: &'a menu::Fsm,
    require_admin: bool,
) -> CheckMenuAccessInput<'a> {
    let supported = matches!(message.chat(), Chat::Private(_) | Chat::Group(_) | Chat::Supergroup(_));
    let user_id = message
        .from()
        .filter(|user| supported && !user.is_bot && message.sender_chat().is_none())
        .map(|user| user.id);
    CheckMenuAccessInput {
        chat_id: message.chat().id(),
        user_id,
        menu_owner_id: None,
        callback_id: None,
        ephemeral_message_id: message.ephemeral_message_id(),
        require_admin,
        chat_cfg,
        fsm,
    }
}

fn callback_access_input<'a>(
    query: &'a CallbackQuery,
    chat_cfg: &'a ChatConfig,
    fsm: &'a menu::Fsm,
    require_admin: bool,
) -> Option<CheckMenuAccessInput<'a>> {
    let message = query.message.as_deref()?;
    let supported = matches!(message.chat(), Chat::Private(_) | Chat::Group(_) | Chat::Supergroup(_));
    Some(CheckMenuAccessInput {
        chat_id: message.chat().id(),
        user_id: (supported && !query.from.is_bot).then_some(query.from.id),
        menu_owner_id: message.receiver_user().map(|user| user.id),
        callback_id: Some(&query.id),
        ephemeral_message_id: message.ephemeral_message_id(),
        require_admin,
        chat_cfg,
        fsm,
    })
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
            send_target(&command),
            menu::MenuTarget::Send {
                chat_id: -1,
                command_message_id: Some(7),
                ephemeral: Some(EphemeralMessage {
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
            send_target(&Message::Text(ephemeral.clone())),
            menu::MenuTarget::Send {
                command_message_id: None,
                ephemeral: Some(EphemeralMessage {
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
                message: MenuMessage::EphemeralMessage {
                    chat_id: -1,
                    receiver_user_id: 2,
                    ephemeral_message_id: 9,
                },
                callback_id: Some("synthetic"),
            })
        ));
        let config = ChatConfig::new(2, true, "en".into());
        let fsm = menu::Fsm::new(MemoryStorage::new(), StorageKey::new(1, -1, 2, None, None));
        let foreign = query.from(User::new(3, false, "Other"));
        let access = callback_access_input(&foreign, &config, &fsm, false).unwrap();
        assert_eq!(access.menu_owner_id, Some(2));
        assert_eq!(access.user_id, Some(3));

        let private = Message::Text(MessageText::new(7, 1, ChatPrivate::new(2), "/start").from(user));
        assert!(matches!(
            send_target(&private),
            menu::MenuTarget::Send {
                ephemeral: None,
                command_message_id: None,
                ..
            }
        ));
    }

    #[test]
    fn unsupported_and_anonymous_menu_senders_are_rejected() {
        let fsm = menu::Fsm::new(MemoryStorage::new(), StorageKey::new(1, -1, 2, None, None));
        let config = ChatConfig::new(2, true, "en".into());
        let user = User::new(2, false, "Owner");
        let message = Message::Text(MessageText::new(7, 1, ChatGroup::new(-1), "/start").from(user.clone()));
        assert_eq!(message_access_input(&message, &config, &fsm, false).user_id, Some(2));
        let anonymous = Message::Text(
            MessageText::new(7, 1, ChatGroup::new(-1), "/start")
                .from(user)
                .sender_chat(ChatGroup::new(-1)),
        );
        assert_eq!(message_access_input(&anonymous, &config, &fsm, false).user_id, None);
    }
}
