use crate::database::TxManager;
use crate::interactors::{downloaded_media, node_router, Interactor as _};
use crate::services::messenger::{MessengerPort, SendTextRequest, TextFormat};
use crate::utils::{format_error_report, ErrorMessageFormatter};

use froodi::{Inject, InjectTransient};
use std::fmt::Write as _;
use telers::utils::text::{html_expandable_blockquote, html_quote};
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
};
use tracing::{error, instrument};

#[instrument(skip_all)]
pub async fn stats<Messenger>(
    message: Message,
    Inject(error_formatter): Inject<ErrorMessageFormatter>,
    Inject(messenger): Inject<Messenger>,
    Inject(get_media_stats): Inject<downloaded_media::GetStats>,
    Inject(node_node_stats): Inject<node_router::GetStats>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let media_stats = get_media_stats
        .execute(downloaded_media::GetStatsInput {
            top_domains_limit: 5,
            tx_manager: &mut tx_manager,
        })
        .await;
    let nodes_stats = node_node_stats.execute(node_router::GetStatsInput {}).await.unwrap_or_default();

    let text = match media_stats {
        Ok((media_stats, chat_stats)) => {
            let mut nodes_text = "- Nodes:\n".to_owned();
            for node_stats in nodes_stats {
                let _ = writeln!(
                    nodes_text,
                    "{}. ({}/{})",
                    html_quote(node_stats.name),
                    node_stats.active_downloads,
                    node_stats.max_concurrent,
                );
            }

            let mut top_domains_text = "- Most used domains:\n".to_owned();
            for (index, top_domain) in media_stats.top_domains.iter().enumerate() {
                let _ = writeln!(
                    top_domains_text,
                    "{}. {} ({} count)",
                    index + 1,
                    top_domain.domain,
                    top_domain.count
                );
            }

            format!(
                "<b>Stats</b>\n\
                - Chats count: {}\n\
                - Downloads last 1/7/30/total days: {}/{}/{}/{} count\n\
                {nodes_text}\
                {top_domains_text}\
                ",
                chat_stats.count,
                media_stats.last_day.count,
                media_stats.last_week.count,
                media_stats.last_month.count,
                media_stats.total.count,
            )
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Get error");
            format!(
                "Sorry, an error to get stats\n{}",
                html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
            )
        }
    };

    messenger
        .send_text(SendTextRequest {
            chat_id: message.chat().id(),
            text: &text,
            reply_to_message_id: message.reply_to_message().as_ref().map(|message| message.message_id()),
            format: Some(TextFormat::Html),
            disable_link_preview: true,
        })
        .await?;

    Ok(EventReturn::Finish)
}
