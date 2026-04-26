use std::{fmt::Write as _, sync::Arc};

use rust_i18n::t;
use telers::{
    errors::HandlerError,
    utils::text::{html_expandable_blockquote, html_quote},
};
use tracing::error;

use crate::{
    database::TxManager,
    entities::ChatConfig,
    interactors::Interactor,
    services::{
        downloaded_media,
        messenger::{MessengerPort, SendTextRequest, TextFormat},
        node_router,
    },
    utils::ErrorFormatter,
};

pub struct Stats<Messenger> {
    pub error_formatter: Arc<ErrorFormatter>,
    pub messenger: Arc<Messenger>,
    pub get_media_stats: Arc<downloaded_media::GetStats>,
    pub get_node_stats: Arc<node_router::GetStats>,
}

pub struct StatsInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub chat_cfg: Option<&'a ChatConfig>,
    pub tx_manager: &'a mut TxManager,
}

impl<Messenger> Interactor<StatsInput<'_>> for &Stats<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: StatsInput<'_>) -> Result<Self::Output, Self::Err> {
        let locale = input.chat_cfg.map_or("en", ChatConfig::locale_str);
        let media_stats = self
            .get_media_stats
            .execute(downloaded_media::GetStatsInput {
                top_domains_limit: 5,
                tx_manager: input.tx_manager,
            })
            .await;
        let nodes_stats = self.get_node_stats.execute(node_router::GetStatsInput {}).await.unwrap_or_default();

        let text = match media_stats {
            Ok((media_stats, chat_stats)) => {
                let mut nodes_text = String::new();
                let _ = writeln!(nodes_text, "- {}", t!("stats.nodes_header", locale = locale));
                for node_stats in nodes_stats {
                    let _ = writeln!(
                        nodes_text,
                        "{}. ({}/{})",
                        html_quote(node_stats.name),
                        node_stats.active_downloads,
                        node_stats.max_concurrent,
                    );
                }

                let mut top_domains_text = String::new();
                let _ = writeln!(top_domains_text, "- {}", t!("stats.top_domains_header", locale = locale));
                for (index, top_domain) in media_stats.top_domains.iter().enumerate() {
                    let count_text = t!("stats.top_domain_count", locale = locale, count = top_domain.count);
                    let _ = writeln!(top_domains_text, "{}. {} ({count_text})", index + 1, top_domain.domain);
                }

                let mut out = String::new();
                let _ = writeln!(out, "{}", t!("stats.header", locale = locale));
                let _ = writeln!(out, "- {}", t!("stats.chats_count", locale = locale, count = chat_stats.count));
                let _ = writeln!(
                    out,
                    "- {}",
                    t!(
                        "stats.downloads",
                        locale = locale,
                        d1 = media_stats.last_day.count,
                        d7 = media_stats.last_week.count,
                        d30 = media_stats.last_month.count,
                        total = media_stats.total.count
                    )
                );
                out.push_str(&nodes_text);
                out.push_str(&top_domains_text);
                out
            }
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Get error");
                format!(
                    "{}\n{}",
                    t!("stats.get_error", locale = locale),
                    html_expandable_blockquote(html_quote(self.error_formatter.format(&err).as_ref()))
                )
            }
        };

        if let Err(err) = self
            .messenger
            .send_text(SendTextRequest {
                chat_id: input.chat_id,
                text: &text,
                reply_to_message_id: input.reply_to_message_id,
                format: Some(TextFormat::Html),
                disable_link_preview: true,
            })
            .await
        {
            error!(err = %self.error_formatter.format(&err), "Send error");
        }

        Ok(())
    }
}
