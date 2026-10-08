use crate::{
    entities::{ChatConfig, ChatConfigExcludeDomains},
    filters::get_host_from_text,
    interactors::{menu, Interactor as _},
    services::messenger::MessengerPort,
    value_objects::menu::{DeleteDomain, OpenScreen, Screen, SetLanguage, SetLinkVisibility},
};

use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::{CallbackQuery, Message},
    Extension,
};
use tracing::instrument;

#[instrument(skip_all)]
pub async fn open_main<Messenger>(
    message: Message,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(menu::OpenMenuInput {
            target: menu::MenuTarget::Send {
                chat_id: message.chat().id(),
            },
            screen: Screen::Main,
            notice: None,
            chat_cfg: &chat_cfg,
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
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(menu::OpenMenuInput {
            target: menu::MenuTarget::Send {
                chat_id: message.chat().id(),
            },
            screen: Screen::Help,
            notice: None,
            chat_cfg: &chat_cfg,
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
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(menu::OpenMenuInput {
            target: menu::MenuTarget::Send {
                chat_id: message.chat().id(),
            },
            screen: Screen::Stats,
            notice: None,
            chat_cfg: &chat_cfg,
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
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(menu::OpenMenuInput {
            target: menu::MenuTarget::Send {
                chat_id: message.chat().id(),
            },
            screen: Screen::Language,
            notice: None,
            chat_cfg: &chat_cfg,
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
            chat_id: message.chat().id(),
            host: host.as_deref(),
            chat_cfg: &chat_cfg,
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
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::OpenMenu<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(message) = query.message.as_deref() else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::OpenMenuInput {
            target: menu::MenuTarget::Edit {
                chat_id: message.chat().id(),
                message_id: message.message_id(),
                callback_id: &query.id,
            },
            screen: data.screen,
            notice: None,
            chat_cfg: &chat_cfg,
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
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::SetMenuLanguage<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(message) = query.message.as_deref() else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::SetMenuLanguageInput {
            target: menu::MenuTarget::Edit {
                chat_id: message.chat().id(),
                message_id: message.message_id(),
                callback_id: &query.id,
            },
            locale: data.locale,
            chat_cfg: &chat_cfg,
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
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::SetMenuLinkVisibility<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(message) = query.message.as_deref() else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::SetMenuLinkVisibilityInput {
            target: menu::MenuTarget::Edit {
                chat_id: message.chat().id(),
                message_id: message.message_id(),
                callback_id: &query.id,
            },
            link_is_visible: data.visible,
            chat_cfg: &chat_cfg,
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
    Extension(exclude_domains): Extension<ChatConfigExcludeDomains>,
    fsm: menu::Fsm,
    Inject(interactor): Inject<menu::RemoveMenuDomain<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let Some(message) = query.message.as_deref() else {
        return Ok(EventReturn::Finish);
    };
    interactor
        .execute(menu::RemoveMenuDomainInput {
            target: menu::MenuTarget::Edit {
                chat_id: message.chat().id(),
                message_id: message.message_id(),
                callback_id: &query.id,
            },
            domain: data.domain,
            chat_cfg: &chat_cfg,
            exclude_domains: &exclude_domains,
            fsm: &fsm,
        })
        .await?;
    Ok(EventReturn::Finish)
}
