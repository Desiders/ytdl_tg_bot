use crate::config::{BotConfig, YtDlpConfig};

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
pub async fn start(
    bot: Bot,
    message: Message,
    Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
    Inject(bot_cfg): Inject<BotConfig>,
) -> HandlerResult {
    let bot_info = bot.send(GetMe {}).await?;
    let text = format!(
        "Hi, {first_name}. I'm a bot that can help you download media from YouTube and other resources.\n\n\
        In a private chat, send me a video link and I will reply with a file.\n\
        In a group chat, send <code>/vd</code> (<code>/video_download</code>) with a link or reply to a message containing a link.\n\
        If you want to download audio, send <code>/ad</code> (<code>/audio_download</code>) instead of <code>/vd</code>.\n\
        You can also use <code>/rv</code> (<code>/random_video</code>) or <code>/ra</code> (<code>/random_audio</code>) to get random media.\n\n\
        Params to download several media from playlist: <code>[items=start:stop:step]</code>.\n\
        Params to select download media language: <code>[lang=ru|en|en-US|en-GB]</code>.\n\
        Params to override used domains for <code>/rv</code> and <code>/ra</code>: <code>[domains=youtube.com|youtu.be]</code>.\n\n\
        You can use me in inline mode in any chat by typing <code>@{bot_username} </code><code>&lt;url&gt;</code>.\n\
        If text is specified instead of a URL, a YouTube video search will be performed.\n\
        * I download videos and audios in the best available quality under {max_file_size_in_mb}MB.\n\
        * The bot is open source, and you can find the source code: {source_code}.",
        first_name = message
            .from()
            .as_ref()
            .map_or("Anonymous".to_owned(), |user| html_quote(user.first_name.as_ref())),
        bot_username = bot_info.username.expect("Bots always have a username"),
        max_file_size_in_mb = yt_dlp_cfg.max_file_size / 1000 / 1000,
        source_code = html_text_link("here", html_quote(&bot_cfg.src_url)),
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
