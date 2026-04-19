use std::{fmt::Write as _, sync::Arc};
use telers::{
    errors::HandlerError,
    utils::text::{html_bold, html_code, html_expandable_blockquote, html_quote},
};
use tracing::error;

use crate::{
    database::TxManager,
    entities::{ChatConfig, ChatConfigExcludeDomain, ChatConfigExcludeDomains, ChatConfigUpdate},
    handlers_utils::progress,
    interactors::Interactor,
    services::{
        chat,
        messenger::{MessengerPort, TextFormat},
    },
    utils::ErrorFormatter,
};

pub struct ChangeLinkVisibility<Messenger> {
    pub error_formatter: Arc<ErrorFormatter>,
    pub messenger: Arc<Messenger>,
    pub update_chat_cfg: Arc<chat::UpdateChatConfig>,
}

pub struct ChangeLinkVisibilityInput<'a> {
    pub reply_to_message_id: Option<i64>,
    pub chat_cfg: &'a ChatConfig,
    pub tx_manager: &'a mut TxManager,
}

impl<Messenger> Interactor<ChangeLinkVisibilityInput<'_>> for &ChangeLinkVisibility<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: ChangeLinkVisibilityInput<'_>) -> Result<Self::Output, Self::Err> {
        let link_is_visible = !input.chat_cfg.link_is_visible;
        let text = match self
            .update_chat_cfg
            .execute(chat::UpdateChatConfigInput {
                dto: ChatConfigUpdate::new(input.chat_cfg.tg_id).with_link_is_visible(link_is_visible),
                tx_manager: input.tx_manager,
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
                error!(err = %self.error_formatter.format(&err), "Update error");
                format!(
                    "Sorry, an error to change link visibility\n{}",
                    html_expandable_blockquote(html_quote(self.error_formatter.format(&err).as_ref()))
                )
            }
        };

        if let Err(err) = progress::new(
            self.messenger.as_ref(),
            &text,
            input.chat_cfg.tg_id,
            input.reply_to_message_id,
            Some(TextFormat::Html),
        )
        .await
        {
            error!(err = %self.error_formatter.format(&err), "Send error");
        }

        Ok(())
    }
}

pub struct AddExcludeDomain<Messenger> {
    pub error_formatter: Arc<ErrorFormatter>,
    pub messenger: Arc<Messenger>,
    pub add_domain: Arc<chat::AddExcludeDomain>,
}

pub struct AddExcludeDomainInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub host: &'a str,
    pub exclude_domains: &'a ChatConfigExcludeDomains,
    pub tx_manager: &'a mut TxManager,
}

impl<Messenger> Interactor<AddExcludeDomainInput<'_>> for &AddExcludeDomain<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: AddExcludeDomainInput<'_>) -> Result<Self::Output, Self::Err> {
        if input.exclude_domains.0.contains(&input.host.to_owned()) {
            if let Err(err) = progress::new(
                self.messenger.as_ref(),
                "Domain already exists in exclude list",
                input.chat_id,
                input.reply_to_message_id,
                None,
            )
            .await
            {
                error!(err = %self.error_formatter.format(&err), "Send error");
            }
            return Ok(());
        }
        if input.exclude_domains.0.len() >= 15 {
            if let Err(err) = progress::new(
                self.messenger.as_ref(),
                "Too many domains in exclude list. Limit is 15.",
                input.chat_id,
                input.reply_to_message_id,
                None,
            )
            .await
            {
                error!(err = %self.error_formatter.format(&err), "Send error");
            }
            return Ok(());
        }

        let host = input.host.to_owned();
        let text = match self
            .add_domain
            .execute(chat::ExcludeDomainInput {
                dto: ChatConfigExcludeDomain::new(input.chat_id, host.clone()),
                tx_manager: input.tx_manager,
            })
            .await
        {
            Ok(()) => {
                let mut current_domains_text = "Current exclude list:\n".to_owned();
                for (index, domain) in input.exclude_domains.0.iter().chain(Some(&host)).enumerate() {
                    let _ = writeln!(current_domains_text, "{}. {}", index + 1, html_code(html_quote(domain)));
                }

                format!(
                    "Domain {} added to exclude list.\n\
                    This host will not be downloaded \"silent\" mode.\n\
                    You can remove it with <code>/rm_exclude_domain</code> command.\n\
                    \n\
                    {current_domains_text}\
                    ",
                    html_code(html_quote(&host)),
                )
            }
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Add error");
                format!(
                    "Sorry, an error to add domain\n{}",
                    html_expandable_blockquote(html_quote(self.error_formatter.format(&err).as_ref()))
                )
            }
        };

        if let Err(err) = progress::new(
            self.messenger.as_ref(),
            &text,
            input.chat_id,
            input.reply_to_message_id,
            Some(TextFormat::Html),
        )
        .await
        {
            error!(err = %self.error_formatter.format(&err), "Send error");
        }
        Ok(())
    }
}

pub struct RemoveExcludeDomain<Messenger> {
    pub error_formatter: Arc<ErrorFormatter>,
    pub messenger: Arc<Messenger>,
    pub remove_domain: Arc<chat::RemoveExcludeDomain>,
}

pub struct RemoveExcludeDomainInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub host: &'a str,
    pub exclude_domains: &'a ChatConfigExcludeDomains,
    pub tx_manager: &'a mut TxManager,
}

impl<Messenger> Interactor<RemoveExcludeDomainInput<'_>> for &RemoveExcludeDomain<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: RemoveExcludeDomainInput<'_>) -> Result<Self::Output, Self::Err> {
        if !input.exclude_domains.0.contains(&input.host.to_owned()) {
            if let Err(err) = progress::new(
                self.messenger.as_ref(),
                "Domain not found in exclude list",
                input.chat_id,
                input.reply_to_message_id,
                None,
            )
            .await
            {
                error!(err = %self.error_formatter.format(&err), "Send error");
            }
            return Ok(());
        }

        let host = input.host.to_owned();
        let text = match self
            .remove_domain
            .execute(chat::ExcludeDomainInput {
                dto: ChatConfigExcludeDomain::new(input.chat_id, host.clone()),
                tx_manager: input.tx_manager,
            })
            .await
        {
            Ok(()) => {
                let mut current_domains_text = "Current exclude list:\n".to_owned();
                for (index, domain) in input.exclude_domains.0.iter().filter(|&domain| *domain != host).enumerate() {
                    let _ = writeln!(current_domains_text, "{}. {}", index + 1, html_code(html_quote(domain)));
                }

                format!(
                    "Domain {} removed from exclude list.\n\
                    You can add it with <code>/add_exclude_domain</code> command.\n\
                    \n\
                    {current_domains_text}\
                    ",
                    html_code(html_quote(&host)),
                )
            }
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Add error");
                format!(
                    "Sorry, an error to remove domain\n{}",
                    html_expandable_blockquote(html_quote(self.error_formatter.format(&err).as_ref()))
                )
            }
        };

        if let Err(err) = progress::new(
            self.messenger.as_ref(),
            &text,
            input.chat_id,
            input.reply_to_message_id,
            Some(TextFormat::Html),
        )
        .await
        {
            error!(err = %self.error_formatter.format(&err), "Send error");
        }
        Ok(())
    }
}
