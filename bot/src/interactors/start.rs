use std::{fmt::Write as _, sync::Arc};

use rust_i18n::t;
use telers::{
    errors::HandlerError,
    utils::text::{html_quote, html_text_link},
};

use crate::{
    config::Config,
    entities::ChatConfig,
    interactors::Interactor,
    locale::Locale,
    services::messenger::{MessengerPort, SendTextRequest, TextFormat},
    utils::ErrorFormatter,
};
use tracing::error;

pub struct Start<Messenger> {
    pub cfg: Arc<Config>,
    pub error_formatter: Arc<ErrorFormatter>,
    pub messenger: Arc<Messenger>,
}

pub struct StartInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub chat_cfg: Option<&'a ChatConfig>,
}

impl<Messenger> Interactor<StartInput<'_>> for &Start<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: StartInput<'_>) -> Result<Self::Output, Self::Err> {
        let username = match self.messenger.username().await {
            Ok(username) => username,
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Get messenger username error");
                return Ok(());
            }
        };

        let locale = input.chat_cfg.map_or(Locale::En, ChatConfig::locale);
        let max_file_size_in_mb = self.cfg.yt_dlp.max_file_size / 1000 / 1000;
        let source_label = t!("start.source_code_label", locale = locale.as_str()).into_owned();
        let source_code = html_text_link(source_label.as_str(), html_quote(&self.cfg.bot.src_url));

        let mut text = String::new();
        let _ = writeln!(text, "{}", t!("start.header_commands", locale = locale.as_str()));
        for key in [
            "start.cmd_vd",
            "start.cmd_ad",
            "start.cmd_random",
            "start.cmd_add_ed",
            "start.cmd_rm_ed",
            "start.cmd_change_link_visibility",
            "start.cmd_stats",
            "start.cmd_lang",
        ] {
            let _ = writeln!(text, "- {}", t!(key, locale = locale.as_str()));
        }
        text.push('\n');
        let _ = writeln!(text, "{}", t!("start.header_inline", locale = locale.as_str()));
        for key in ["start.inline_url", "start.inline_search"] {
            let _ = writeln!(text, "- {}", t!(key, locale = locale.as_str(), username = username.as_str()));
        }
        text.push('\n');
        let _ = writeln!(text, "{}", t!("start.header_arguments", locale = locale.as_str()));
        let _ = writeln!(text, "{}", t!("start.args_for_vd_ad", locale = locale.as_str()));
        for key in ["start.arg_lang", "start.arg_items", "start.arg_crop"] {
            let _ = writeln!(text, "  - {}", t!(key, locale = locale.as_str()));
        }
        let _ = writeln!(text, "{}", t!("start.args_for_random", locale = locale.as_str()));
        let _ = writeln!(text, "  - {}", t!("start.arg_domains", locale = locale.as_str()));
        text.push('\n');
        let _ = writeln!(text, "{}", t!("start.header_notes", locale = locale.as_str()));
        for key in [
            "start.note_args",
            "start.note_optional",
            "start.note_websites",
            "start.note_inline",
            "start.note_ignore",
        ] {
            let _ = writeln!(text, "- {}", t!(key, locale = locale.as_str()));
        }
        let _ = writeln!(
            text,
            "- {}",
            t!(
                "start.note_max_size",
                locale = locale.as_str(),
                max_file_size_in_mb = max_file_size_in_mb
            )
        );
        let _ = writeln!(
            text,
            "- {}",
            t!("start.note_open_source", locale = locale.as_str(), source_code = source_code)
        );

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
