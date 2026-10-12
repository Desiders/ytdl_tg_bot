use std::{sync::Arc, time::Duration};

use proto::downloader::{MediaInfoRequest, PostEntry};
use rust_i18n::t;
use telers::{
    errors::HandlerError,
    utils::text::{html_expandable_blockquote, html_quote, html_text_link},
};
use tracing::{debug, error, instrument};
use url::Url;

use crate::{
    entities::{DownloadJob, JobTarget},
    handlers_utils::progress,
    interactors::Interactor,
    services::{
        messenger::{EditCaptionRequest, EditTarget, EditTextRequest, MessengerPort, SendTextRequest, SentMessage, TextFormat},
        node_router::{get_media_info, NodeRouter},
    },
    utils::ErrorFormatter,
};

const BODY_UNITS: usize = 4_096;
const CAPTION_UNITS: usize = 1_024;
const POST_INFO_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Display<Messenger> {
    node_router: Arc<NodeRouter>,
    messenger: Arc<Messenger>,
    error_formatter: Arc<ErrorFormatter>,
}

pub struct DisplayInput<'a> {
    pub job: &'a DownloadJob,
    pub message: Option<&'a SentMessage>,
    pub has_media: bool,
}

impl<Messenger> Display<Messenger> {
    #[must_use]
    pub const fn new(node_router: Arc<NodeRouter>, messenger: Arc<Messenger>, error_formatter: Arc<ErrorFormatter>) -> Self {
        Self {
            node_router,
            messenger,
            error_formatter,
        }
    }
}

impl<Messenger> Interactor<DisplayInput<'_>> for &Display<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = bool;
    type Err = HandlerError;

    #[allow(clippy::too_many_lines)]
    #[instrument(skip_all, fields(job_id = %input.job.job_id, ?input.job.params))]
    async fn execute(self, input: DisplayInput<'_>) -> Result<Self::Output, Self::Err> {
        let job = input.job;
        let Some(url) = job.url.as_ref() else { return Ok(false) };
        let posts = match tokio::time::timeout(
            POST_INFO_TIMEOUT,
            get_media_info(
                &self.node_router,
                url.domain(),
                MediaInfoRequest {
                    url: url.to_string(),
                    audio_language: job.params.0.get("lang").cloned().unwrap_or_default(),
                    media_type: "post".to_owned(),
                    max_file_size: self.node_router.max_file_size(),
                    ..Default::default()
                },
            ),
        )
        .await
        {
            Ok(Ok(response)) if response.posts.iter().any(|post| !post.text.trim().is_empty()) => response.posts,
            Ok(Err(err)) => {
                debug!(err = %self.error_formatter.format(&err), "Post metadata is unavailable");
                return Ok(false);
            }
            Err(err) => {
                debug!(%err, "Post metadata timed out");
                return Ok(false);
            }
            Ok(Ok(_)) => return Ok(false),
        };
        let limit = if input.has_media { CAPTION_UNITS } else { BODY_UNITS };
        let text = render(&posts, url, limit, job.link_is_visible);
        let delivered = match &job.target {
            JobTarget::Command { chat_id, message_id } => {
                if input.has_media {
                    let Some(message) = input.message else { return Ok(false) };
                    self.messenger
                        .edit_caption(EditCaptionRequest {
                            target: EditTarget::ChatMessage {
                                chat_id: *chat_id,
                                message_id: message.message_id,
                            },
                            text: &text,
                            format: Some(TextFormat::Html),
                            clear_inline_keyboard: true,
                        })
                        .await
                } else {
                    self.messenger
                        .send_text(SendTextRequest {
                            chat_id: *chat_id,
                            reply_to_message_id: Some(*message_id),
                            text: &text,
                            format: Some(TextFormat::Html),
                            disable_link_preview: true,
                        })
                        .await
                        .map(|_| ())
                }
            }
            JobTarget::Inline { inline_message_id, .. } => {
                let target = EditTarget::InlineMessage { inline_message_id };
                if input.has_media {
                    self.messenger
                        .edit_caption(EditCaptionRequest {
                            target,
                            text: &text,
                            format: Some(TextFormat::Html),
                            clear_inline_keyboard: true,
                        })
                        .await
                } else {
                    self.messenger
                        .edit_text(EditTextRequest {
                            is_progress: false,
                            target,
                            text: &text,
                            format: Some(TextFormat::Html),
                            disable_link_preview: true,
                            clear_inline_keyboard: true,
                        })
                        .await
                }
            }
        };
        if let Err(err) = delivered {
            let formatted = self.error_formatter.format(&err);
            error!(err = %formatted, "Display post text error");
            if !input.has_media {
                if !job.quiet {
                    let text = format!(
                        "{}\n{}",
                        t!("download.error_send_message", locale = job.chat_cfg.locale().as_str()),
                        html_expandable_blockquote(html_quote(formatted.as_ref()))
                    );
                    match &job.target {
                        JobTarget::Command { chat_id, message_id } => {
                            let _ =
                                progress::new(self.messenger.as_ref(), &text, *chat_id, Some(*message_id), Some(TextFormat::Html)).await;
                        }
                        JobTarget::Inline { inline_message_id, .. } => {
                            let _ = progress::is_error_in_chosen_inline(
                                self.messenger.as_ref(),
                                inline_message_id,
                                &text,
                                Some(TextFormat::Html),
                            )
                            .await;
                        }
                    }
                }
                // A known post delivery failure must not fall back to media extraction.
                return Err(HandlerError::from_display(formatted));
            }
        }
        Ok(true)
    }
}

fn author_label(post: &PostEntry) -> String {
    let author = post.author.as_deref().unwrap_or_default();
    let handle = post.author_handle.as_deref().unwrap_or_default().trim_start_matches('@');
    let label = match (author.is_empty(), handle.is_empty()) {
        (false, false) => format!("{author} (@{handle})"),
        (false, true) => author.to_owned(),
        (true, false) => format!("@{handle}"),
        (true, true) => String::new(),
    };
    label.replace(['\n', '\r'], " ").chars().take(80).collect()
}

fn source_url(post: &PostEntry, fallback: &Url) -> Url {
    Url::parse(&post.webpage_url)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or_else(|| fallback.clone())
}

fn render(posts: &[PostEntry], fallback: &Url, limit: usize, link_is_visible: bool) -> String {
    let footer = if link_is_visible {
        html_text_link("Link", html_quote(source_url(&posts[0], fallback).as_str()))
    } else {
        String::new()
    };
    let body_limit = limit - if link_is_visible { "Link".len() + 2 } else { 0 };
    let (main, units) = render_entry(&posts[0], false, fallback, body_limit, link_is_visible).unwrap_or_default();
    let mut remaining = body_limit - units;
    let mut parts = Vec::new();
    for post in posts.iter().skip(1) {
        let separator = if main.is_empty() && parts.is_empty() { 0 } else { 2 };
        let Some((html, units)) = render_entry(post, true, fallback, remaining.saturating_sub(separator), link_is_visible) else {
            break;
        };
        remaining -= units + separator;
        parts.push(html);
    }
    if !main.is_empty() {
        parts.push(main);
    }
    if !footer.is_empty() {
        parts.push(footer);
    }
    parts.join("\n\n")
}

fn render_entry(post: &PostEntry, quoted: bool, fallback: &Url, limit: usize, link_is_visible: bool) -> Option<(String, usize)> {
    let label = author_label(post);
    let url = source_url(post, fallback);
    let header = if quoted && !label.is_empty() {
        format!("<b>{}</b>\n", html_quote(&label))
    } else {
        String::new()
    };
    let footer = if quoted && link_is_visible && url != *fallback {
        format!("\n{}", html_text_link("Link", html_quote(url.as_str())))
    } else {
        String::new()
    };
    let overhead =
        if header.is_empty() { 0 } else { label.encode_utf16().count() + 1 } + if footer.is_empty() { 0 } else { "\nLink".len() };
    if overhead >= limit {
        return None;
    }
    let text = truncate_text(&post.text, limit - overhead);
    let units = text.encode_utf16().count() + overhead;
    let body = format!("{header}{}{footer}", html_quote(&text));
    let html = if post.text.encode_utf16().count() > CAPTION_UNITS || text != post.text {
        html_expandable_blockquote(body)
    } else if quoted {
        format!("<blockquote>{body}</blockquote>")
    } else {
        body
    };
    Some((html, units))
}

fn truncate_text(text: &str, limit: usize) -> String {
    if text.encode_utf16().count() <= limit {
        return text.to_owned();
    }
    let mut units = 0;
    let mut truncated = String::new();
    for ch in text.chars() {
        if units + ch.len_utf16() >= limit {
            break;
        }
        truncated.push(ch);
        units += ch.len_utf16();
    }
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_text_is_escaped_and_truncated_with_expandable_quotes_and_source_links() {
        let url = Url::parse("https://source.example.invalid/post?one=1&two=2").unwrap();
        let main = PostEntry {
            text: "Hello <world> & friends".into(),
            author: Some("Author".into()),
            ..Default::default()
        };
        let link = "<a href=\"https://source.example.invalid/post?one=1&amp;two=2\">Link</a>";
        let footer = format!("\n\n{link}");
        let text = render(std::slice::from_ref(&main), &url, CAPTION_UNITS, true);
        assert_eq!(text, format!("Hello &lt;world&gt; &amp; friends{footer}"));

        let quote = PostEntry {
            text: "😀<&>".repeat(2_000),
            author: Some("Quoted".into()),
            author_handle: Some("quoted_handle".into()),
            webpage_url: "https://source.example.invalid/context?one=1&two=2".into(),
        };
        let text = render(&[main.clone(), quote.clone()], &url, CAPTION_UNITS, true);
        assert!(text.starts_with("<blockquote expandable><b>Quoted (@quoted_handle)</b>"));
        assert!(text.contains("…\n<a href=\"https://source.example.invalid/context?one=1&amp;two=2\">Link</a></blockquote>"));
        assert!(text.ends_with(&format!("Hello &lt;world&gt; &amp; friends{footer}")));
        let hidden = render(&[main.clone(), quote], &url, CAPTION_UNITS, false);
        assert!(!hidden.contains("<a "));
        assert!(hidden.starts_with("<blockquote expandable><b>Quoted (@quoted_handle)</b>"));
        assert!(hidden.ends_with("Hello &lt;world&gt; &amp; friends"));

        let mut long = main;
        long.text = "😀<&>".repeat(2_000);
        let text = render(std::slice::from_ref(&long), &url, CAPTION_UNITS, true);
        assert_eq!(
            text,
            format!(
                "{}{footer}",
                html_expandable_blockquote(html_quote(format!("{}😀…", "😀<&>".repeat(203))))
            )
        );
        long.text = "😀".repeat(3_000);
        let text = render(std::slice::from_ref(&long), &url, BODY_UNITS, true);
        assert_eq!(
            text,
            format!("{}{footer}", html_expandable_blockquote(format!("{}…", "😀".repeat(2_044))))
        );

        long.text = "Long text ".repeat(150);
        let text = render(std::slice::from_ref(&long), &url, BODY_UNITS, true);
        assert_eq!(text, format!("{}{footer}", html_expandable_blockquote(&long.text)));

        long.author = None;
        let text = render(std::slice::from_ref(&long), &url, BODY_UNITS, true);
        assert_eq!(text, format!("{}\n\n{link}", html_expandable_blockquote(&long.text)));
        assert_eq!(
            render(std::slice::from_ref(&long), &url, BODY_UNITS, false),
            html_expandable_blockquote(&long.text)
        );
    }
}
