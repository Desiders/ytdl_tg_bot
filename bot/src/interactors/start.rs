use std::sync::Arc;

use telers::{
    errors::HandlerError,
    utils::text::{html_quote, html_text_link},
};

use crate::{
    config::Config,
    interactors::Interactor,
    services::messenger::{MessengerPort, SendTextRequest, TextFormat},
};

pub struct Start<Messenger> {
    pub cfg: Arc<Config>,
    pub messenger: Arc<Messenger>,
}

pub struct StartInput {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
}

impl<Messenger> Interactor<StartInput> for &Start<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: StartInput) -> Result<Self::Output, Self::Err> {
        let username = self.messenger.username().await?;
        let text = format!(
            "<b>Commands</b>\n\
            - <code>/vd</code> — download video. Calling this command is required to display download progress; otherwise, downloading will occur in \"silent\" mode.\n\
            - <code>/ad</code> — download audio\n\
            - <code>/rv</code>, <code>/ra</code> — random video or audio\n\
            - <code>/add_ed</code> — exclude domain from download\n\
            - <code>/rm_ed</code> — include domain in download\n\
            - <code>/change_link_visibility</code> — change link visibility in media caption\n\
            - <code>/stats</code> — usage statistics\n\
            \n\
            <b>Inline Mode</b>\n\
            - <code>@{username} &lt;url&gt;</code> — download by link\n\
            - <code>@{username} &lt;title&gt;</code> — search on YouTube\n\
            \n\
            <b>Arguments</b>\n\
            For <code>/vd</code> and <code>/ad</code>:\n\
              - lang: Preferred audio language, example: <code>/vd [lang=ru]</code>\n\
              - items: Playlist download, <code>start:end:step</code> (default: 1 for each argument, max: 10 media per command), example: <code>/vd [items=1:3:1]</code>\n\
              - crop: Download only a specific media time range, format <code>start-end</code>, supports <code>hh:mm:ss</code> (default: 0 for start and empty for end), example: <code>/vd [clip=00:01:30-]</code>
            \n\
            For <code>/rv</code> and <code>/ra</code>:\n\
              - domains: Sources separated by <code>|</code>, example: <code>/rv [domains=youtube.com|youtu.be]</code>\n\
            \n\
            <b>Notes</b>\n\
            - Arguments are specified in square brackets separated by commas: <code>[arg=value,arg2=value]</code>;\n\
            - All arguments are optional;\n\
            - Thousands of websites are supported;\n\
            - Inline mode supports <code>lang</code>, but doesn't support <code>items</code>;\n\
            - You can add <code>yv2t_bot=false</code> to a link to ignore it;\n\
            - I download media in the best quality under {max_file_size_in_mb}MB;\n\
            - The bot is open source: {source_code}.\
            ",
            max_file_size_in_mb = self.cfg.yt_dlp.max_file_size / 1000 / 1000,
            source_code = html_text_link("source code", html_quote(&self.cfg.bot.src_url)),
        );

        self.messenger
            .send_text(SendTextRequest {
                chat_id: input.chat_id,
                text: &text,
                reply_to_message_id: input.reply_to_message_id,
                format: Some(TextFormat::Html),
                disable_link_preview: true,
            })
            .await?;

        Ok(())
    }
}
