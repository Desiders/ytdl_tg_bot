use crate::database::TxManager;
use crate::interactors::{downloaded_media, Interactor as _};
use crate::utils::{format_error_report, FormatErrorToMessage as _};

use froodi::{Inject, InjectTransient};
use telers::utils::text::{html_expandable_blockquote, html_quote};
use telers::{
    enums::ParseMode,
    event::{telegram::HandlerResult, EventReturn},
    methods::SendMessage,
    types::{LinkPreviewOptions, Message, ReplyParameters},
    Bot,
};
use tracing::{error, instrument};

#[instrument(skip_all)]
pub async fn stats(
    bot: Bot,
    message: Message,
    Inject(get_stats): Inject<downloaded_media::GetStats>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    let text = match get_stats
        .execute(downloaded_media::GetStatsInput {
            top_domains_limit: 5,
            tx_manager: &mut tx_manager,
        })
        .await
    {
        Ok((media_stats, chat_stats)) => {
            let mut top_domains_text = "- Most used domains:\n".to_owned();
            for (index, top_domain) in media_stats.top_domains.iter().enumerate() {
                top_domains_text.push_str(&format!("{}. {} ({} count)\n", index + 1, top_domain.domain, top_domain.count));
            }

            format!(
                "<b>Stats</b>\n\
                - Chats count: {}\n\
                - Downloads last 1/7/30/total days: {}/{}/{}/{} count\n\
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
            error!(err = format_error_report(&err), "Get err");
            format!(
                "Sorry, an error to get stats\n{}",
                html_expandable_blockquote(html_quote(err.format(&bot.token)))
            )
        }
    };

    bot.send(
        SendMessage::new(message.chat().id(), text)
            .parse_mode(ParseMode::HTML)
            .link_preview_options(LinkPreviewOptions::new().is_disabled(true))
            .reply_parameters_option(
                message
                    .reply_to_message()
                    .as_ref()
                    .map(|message| ReplyParameters::new(message.id()).allow_sending_without_reply(true)),
            ),
    )
    .await?;

    Ok(EventReturn::Finish)
}
