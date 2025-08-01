use crate::config::{BotConfig, YtDlpConfig};

use telers::{
    enums::ParseMode,
    event::{telegram::HandlerResult, EventReturn},
    methods::{GetMe, SendMessage},
    types::{LinkPreviewOptions, Message, ReplyParameters},
    utils::text::{html_quote, html_text_link},
    Bot, Extension,
};

pub async fn start(
    bot: Bot,
    message: Message,
    Extension(yt_dlp_cfg): Extension<YtDlpConfig>,
    Extension(bot_cfg): Extension<BotConfig>,
) -> HandlerResult {
    let bot_info = bot.send(GetMe {}).await?;
    let text = format!(
        "Hi, {first_name}. I'm a bot that can help you download videos from YouTube.\n\n\
        In a private chat, send me a video link and I will reply with a video or playlist.\n\
        In a group chat, send <code>/vd</code> (<code>/video_download</code>) with a link or reply to the message with a link.\n\n\
        If you want to download an audio, send <code>/ad</code> (<code>/audio_download</code>) instead of <code>/vd</code>. \
        This command works the same way as previous.\n\n\
        Playlist range download: <code>&lt;url&gt; [items=start:stop:step]</code>.\n\
        Select language: <code>&lt;url&gt; [lang=ru|en|en-US|en-GB]</code>.\n\n\
        You can use me in inline mode in any chat by typing <code>@{bot_username} </code><code>&lt;url&gt;</code>.\n\
        If text is specified instead of the URL, a YouTube video search will be performed.\n\
        * You can't download playlists in inline mode.\n\n\
        * I'm download videos and audios in the best quality that less than {max_file_size_in_mb}MB.\n\
        * The bot is open source, and you can find the source code {source_code}.",
        first_name = message
            .from()
            .as_ref()
            .map_or("Anonymous".to_owned(), |user| html_quote(user.first_name.as_ref())),
        bot_username = bot_info.username.expect("Bots always have a username"),
        max_file_size_in_mb = yt_dlp_cfg.max_file_size / 1000 / 1000,
        source_code = html_text_link("here", html_quote(bot_cfg.src_url)),
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
