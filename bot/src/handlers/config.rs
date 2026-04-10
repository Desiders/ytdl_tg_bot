use crate::database::TxManager;
use crate::entities::{ChatConfig, ChatConfigExcludeDomain, ChatConfigExcludeDomains, ChatConfigUpdate};
use crate::handlers_utils::progress;
use crate::interactors::{chat, Interactor as _};
use crate::services::messenger::{MessengerPort, TextFormat};
use crate::utils::{format_error_report, ErrorMessageFormatter};

use froodi::{Inject, InjectTransient};
use std::fmt::Write as _;
use telers::utils::text::{html_bold, html_code};
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
    utils::text::{html_expandable_blockquote, html_quote},
    Extension,
};
use tracing::{error, instrument};
use url::Host;

pub async fn change_link_visibility<Messenger>(
    message: Message,
    Extension(chat_cfg): Extension<ChatConfig>,
    Inject(error_formatter): Inject<ErrorMessageFormatter>,
    Inject(messenger): Inject<Messenger>,
    Inject(update_chat_cfg): Inject<chat::UpdateChatConfig>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let link_is_visible = !chat_cfg.link_is_visible;

    let text = match update_chat_cfg
        .execute(chat::UpdateChatConfigInput {
            dto: ChatConfigUpdate::new(chat_cfg.tg_id).with_link_is_visible(link_is_visible),
            tx_manager: &mut tx_manager,
        })
        .await
    {
        Ok(chat_cfg) => {
            format!(
                "Link visibility has been changed to {}",
                html_bold(if chat_cfg.link_is_visible { "visible" } else { "hidden" }),
            )
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Update error");
            format!(
                "Sorry, an error to change link visibility\n{}",
                html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
            )
        }
    };

    progress::new(
        &*messenger,
        &text,
        chat_cfg.tg_id,
        message.reply_to_message().as_ref().map(|message| message.message_id()),
        Some(TextFormat::Html),
    )
    .await?;

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(%message_id = message.message_id(), %host))]
pub async fn add_exclude_domain<Messenger>(
    message: Message,
    Extension(chat_cfg_domains): Extension<ChatConfigExcludeDomains>,
    Extension(host): Extension<Host>,
    Inject(error_formatter): Inject<ErrorMessageFormatter>,
    Inject(messenger): Inject<Messenger>,
    Inject(add_domain): Inject<chat::AddExcludeDomain>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let chat_id = message.chat().id();
    let host = host.to_string();

    if chat_cfg_domains.0.contains(&host) {
        progress::new(
            &*messenger,
            "Domain already exists in exclude list",
            chat_id,
            message.reply_to_message().as_ref().map(|message| message.message_id()),
            None,
        )
        .await?;
        return Ok(EventReturn::Finish);
    }
    if chat_cfg_domains.0.len() >= 15 {
        progress::new(
            &*messenger,
            "Too many domains in exclude list. Limit is 15.",
            chat_id,
            message.reply_to_message().as_ref().map(|message| message.message_id()),
            None,
        )
        .await?;
        return Ok(EventReturn::Finish);
    }

    let text = match add_domain
        .execute(chat::ExcludeDomainInput {
            dto: ChatConfigExcludeDomain::new(chat_id, host.clone()),
            tx_manager: &mut tx_manager,
        })
        .await
    {
        Ok(()) => {
            let mut current_domains_text = "Current exclude list:\n".to_owned();
            for (index, domain) in chat_cfg_domains.0.iter().chain(Some(&host)).enumerate() {
                let _ = writeln!(current_domains_text, "{}. {}", index + 1, html_code(html_quote(domain)));
            }

            format!(
                "Domain {} added to exclude list.\n\
                This host will not be downloaded \"silent\" mode.\n\
                You can remove it with <code>/rm_exclude_domain</code> command.\n\
                \n\
                {current_domains_text}\
                ",
                html_code(html_quote(host)),
            )
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Add error");
            format!(
                "Sorry, an error to add domain\n{}",
                html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
            )
        }
    };

    progress::new(
        &*messenger,
        &text,
        chat_id,
        message.reply_to_message().as_ref().map(|message| message.message_id()),
        Some(TextFormat::Html),
    )
    .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(%message_id = message.message_id(), %host))]
pub async fn remove_exclude_domain<Messenger>(
    message: Message,
    Extension(chat_cfg_domains): Extension<ChatConfigExcludeDomains>,
    Extension(host): Extension<Host>,
    Inject(error_formatter): Inject<ErrorMessageFormatter>,
    Inject(messenger): Inject<Messenger>,
    Inject(remove_domain): Inject<chat::RemoveExcludeDomain>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let chat_id = message.chat().id();
    let host = host.to_string();

    if !chat_cfg_domains.0.contains(&host) {
        progress::new(
            &*messenger,
            "Domain not found in exclude list",
            chat_id,
            message.reply_to_message().as_ref().map(|message| message.message_id()),
            None,
        )
        .await?;
        return Ok(EventReturn::Finish);
    }

    let text = match remove_domain
        .execute(chat::ExcludeDomainInput {
            dto: ChatConfigExcludeDomain::new(chat_id, host.clone()),
            tx_manager: &mut tx_manager,
        })
        .await
    {
        Ok(()) => {
            let mut current_domains_text = "Current exclude list:\n".to_owned();
            for (index, domain) in chat_cfg_domains.0.iter().filter(|&domain| *domain != host).enumerate() {
                let _ = writeln!(current_domains_text, "{}. {}", index + 1, html_code(html_quote(domain)));
            }

            format!(
                "Domain {} removed from exclude list.\n\
                You can add it with <code>/add_exclude_domain</code> command.\n\
                \n\
                {current_domains_text}\
                ",
                html_code(html_quote(host)),
            )
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Add error");
            format!(
                "Sorry, an error to remove domain\n{}",
                html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
            )
        }
    };

    progress::new(
        &*messenger,
        &text,
        chat_id,
        message.reply_to_message().as_ref().map(|message| message.message_id()),
        Some(TextFormat::Html),
    )
    .await?;
    Ok(EventReturn::Finish)
}
