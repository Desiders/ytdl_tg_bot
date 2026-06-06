use std::{fmt::Write as _, sync::Arc};

use rust_i18n::t;
use telers::{
    errors::HandlerError,
    utils::text::{html_expandable_blockquote, html_quote},
};
use tracing::error;

use crate::{
    entities::ChatConfig,
    interactors::Interactor,
    locale::Locale,
    services::{
        downloaded_media,
        messenger::{MessengerPort, SendTextRequest, TextFormat},
        node_router,
    },
    utils::ErrorFormatter,
    value_objects::ChatType,
};

pub struct Stats<Messenger> {
    error_formatter: Arc<ErrorFormatter>,
    messenger: Arc<Messenger>,
    media_stats: Arc<downloaded_media::GetStats>,
    node_stats: Arc<node_router::GetStats>,
}

impl<Messenger> Stats<Messenger> {
    #[must_use]
    pub const fn new(
        error_formatter: Arc<ErrorFormatter>,
        messenger: Arc<Messenger>,
        media_stats: Arc<downloaded_media::GetStats>,
        node_stats: Arc<node_router::GetStats>,
    ) -> Self {
        Self {
            error_formatter,
            messenger,
            media_stats,
            node_stats,
        }
    }
}

pub struct StatsInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub chat_cfg: Option<&'a ChatConfig>,
}

impl<Messenger> Interactor<StatsInput<'_>> for &Stats<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: StatsInput<'_>) -> Result<Self::Output, Self::Err> {
        let locale = input.chat_cfg.map_or(Locale::En, ChatConfig::locale).as_str();
        let media_stats = self
            .media_stats
            .execute(downloaded_media::GetStatsInput { top_domains_limit: 5 })
            .await;
        let nodes_stats = self.node_stats.execute(node_router::GetStatsInput {}).await.unwrap_or_default();

        let text = match media_stats {
            Ok((media_stats, chat_stats)) => {
                let mut chat_types = String::new();
                for chat_type_count in &chat_stats.by_type {
                    let type_label = match chat_type_count.kind {
                        Some(ChatType::Private) => t!("stats.chat_type_private", locale = locale),
                        Some(ChatType::Group) => t!("stats.chat_type_group", locale = locale),
                        Some(ChatType::Supergroup) => t!("stats.chat_type_supergroup", locale = locale),
                        Some(ChatType::Channel) => t!("stats.chat_type_channel", locale = locale),
                        None => t!("stats.chat_type_unknown", locale = locale),
                    };
                    let _ = writeln!(
                        chat_types,
                        "{}",
                        t!(
                            "stats.chat_type_line",
                            locale = locale,
                            chat_type = type_label,
                            count = chat_type_count.count,
                        )
                    );
                }

                let mut nodes = String::new();
                for node_stats in nodes_stats {
                    let _ = writeln!(
                        nodes,
                        "{}",
                        t!(
                            "stats.node_line",
                            locale = locale,
                            name = html_quote(node_stats.name),
                            active = node_stats.active_downloads,
                            max = node_stats.max_concurrent,
                        )
                    );
                }

                let mut top_domains = String::new();
                for (index, top_domain) in media_stats.top_domains.iter().enumerate() {
                    let _ = writeln!(
                        top_domains,
                        "{}",
                        t!(
                            "stats.top_domain_line",
                            locale = locale,
                            index = index + 1,
                            domain = top_domain.domain,
                            count = top_domain.count,
                        )
                    );
                }

                t!(
                    "stats.body",
                    locale = locale,
                    chats_count = chat_stats.count,
                    chat_types = chat_types,
                    d1 = media_stats.last_day.count,
                    d7 = media_stats.last_week.count,
                    d30 = media_stats.last_month.count,
                    total = media_stats.total.count,
                    nodes = nodes,
                    top_domains = top_domains,
                )
                .into_owned()
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
