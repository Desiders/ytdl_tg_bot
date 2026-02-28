use crate::config::Config;

use froodi::Inject;
use telers::{
    enums::ParseMode,
    event::{telegram::HandlerResult, EventReturn},
    methods::{GetMe, SendMessage},
    types::{LinkPreviewOptions, Message, ReplyParameters},
    utils::text::{html_quote, html_text_link},
    Bot,
};
use tracing::instrument;

#[instrument(skip_all)]
pub async fn start(bot: Bot, message: Message, Inject(cfg): Inject<Config>) -> HandlerResult {
    let bot_info = bot.send(GetMe {}).await?;
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
        - <code>@{bot_username} &lt;url&gt;</code> — download by link\n\
        - <code>@{bot_username} &lt;title&gt;</code> — search on YouTube\n\
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
        bot_username = bot_info.username.expect("Bots always have a username"),
        max_file_size_in_mb = cfg.yt_dlp.max_file_size / 1000 / 1000,
        source_code = html_text_link("source code", html_quote(&cfg.bot.src_url)),
    );

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
