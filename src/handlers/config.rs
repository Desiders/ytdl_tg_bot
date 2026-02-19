use crate::database::TxManager;
use crate::entities::{ChatConfigExcludeDomain, ChatConfigExcludeDomains};
use crate::handlers_utils::progress;
use crate::interactors::{chat, Interactor as _};
use crate::utils::{format_error_report, FormatErrorToMessage as _};

use froodi::{Inject, InjectTransient};
use telers::utils::text::html_code;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
    utils::text::{html_expandable_blockquote, html_quote},
    Bot, Extension,
};
use tracing::{error, instrument};
use url::Host;

#[instrument(skip_all, fields(%message_id = message.id(), %host))]
pub async fn add_exclude_domain(
    bot: Bot,
    message: Message,
    Extension(chat_cfg_domains): Extension<ChatConfigExcludeDomains>,
    Extension(host): Extension<Host>,
    Inject(add_domain): Inject<chat::AddExcludeDomain>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    let chat_id = message.chat().id();
    let host = host.to_string();

    if chat_cfg_domains.0.contains(&host) {
        progress::new(
            &bot,
            "Domain already exists in exclude list",
            chat_id,
            message.reply_to_message().as_ref().map(|message| message.id()),
        )
        .await?;
        return Ok(EventReturn::Finish);
    }
    if chat_cfg_domains.0.len() >= 15 {
        progress::new(
            &bot,
            "Too many domains in exclude list. Limit is 15.",
            chat_id,
            message.reply_to_message().as_ref().map(|message| message.id()),
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
                current_domains_text.push_str(&format!("{}. {}\n", index + 1, html_code(html_quote(domain))));
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
            error!(err = format_error_report(&err), "Add err");
            format!(
                "Sorry, an error to add domain\n{}",
                html_expandable_blockquote(html_quote(err.format(&bot.token)))
            )
        }
    };

    progress::new(
        &bot,
        &text,
        chat_id,
        message.reply_to_message().as_ref().map(|message| message.id()),
    )
    .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(%message_id = message.id(), %host))]
pub async fn remove_exclude_domain(
    bot: Bot,
    message: Message,
    Extension(chat_cfg_domains): Extension<ChatConfigExcludeDomains>,
    Extension(host): Extension<Host>,
    Inject(remove_domain): Inject<chat::RemoveExcludeDomain>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    let chat_id = message.chat().id();
    let host = host.to_string();

    if !chat_cfg_domains.0.contains(&host) {
        progress::new(
            &bot,
            "Domain not found in exclude list",
            chat_id,
            message.reply_to_message().as_ref().map(|message| message.id()),
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
                current_domains_text.push_str(&format!("{}. {}\n", index + 1, html_code(html_quote(domain))));
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
            error!(err = format_error_report(&err), "Add err");
            format!(
                "Sorry, an error to remove domain\n{}",
                html_expandable_blockquote(html_quote(err.format(&bot.token)))
            )
        }
    };

    progress::new(
        &bot,
        &text,
        chat_id,
        message.reply_to_message().as_ref().map(|message| message.id()),
    )
    .await?;
    Ok(EventReturn::Finish)
}
