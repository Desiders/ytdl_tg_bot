use std::{convert::Infallible, future::Future};
use telers::{
    filters::Filter,
    types::{Message, MessageEntity},
    FilterResult, Request,
};

#[derive(Clone)]
pub struct GuestMention {
    bot_id: i64,
    username: String,
}

impl GuestMention {
    #[must_use]
    pub const fn new(bot_id: i64, username: String) -> Self {
        Self { bot_id, username }
    }

    fn matches(&self, message: &Message) -> bool {
        let (text, entities) = if let Some(text) = message.text() {
            (text, message.entities())
        } else if let Some(caption) = message.caption() {
            (caption, message.caption_entities())
        } else {
            return false;
        };
        entities.unwrap_or_default().iter().any(|entity| match entity {
            MessageEntity::TextMention(mention) => mention.user.id == self.bot_id,
            MessageEntity::Mention(mention) => {
                let (Ok(offset), Ok(length)) = (usize::try_from(mention.offset), usize::try_from(mention.length)) else {
                    return false;
                };
                let Some(end) = offset.checked_add(length) else {
                    return false;
                };
                let text: Vec<u16> = text.encode_utf16().collect();
                let Some(mention) = text.get(offset..end) else {
                    return false;
                };
                String::from_utf16_lossy(mention)
                    .strip_prefix('@')
                    .is_some_and(|username| username.eq_ignore_ascii_case(&self.username))
            }
            _ => false,
        })
    }
}

impl Filter for GuestMention {
    type Error = Infallible;

    fn check(&mut self, request: &mut Request) -> impl Future<Output = FilterResult<Self::Error>> {
        let result = request.update.guest_message().is_some_and(|message| self.matches(message));
        async move { Ok(result) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::text_contains_url_with_reply;
    use std::sync::Arc;
    use telers::{
        event::{
            bases::PropagateEventResult,
            telegram::{Handler, Observer},
            EventReturn,
        },
        types::{
            ChatPrivate, MessageEntityMention, MessageEntityTextMention, MessagePhoto, MessageText, PhotoSize, Update, UpdateGuestMessage,
            User,
        },
        Bot, Context, Extensions,
    };
    use url::Url;

    fn message(text: &str) -> MessageText {
        MessageText::new(2, 1, ChatPrivate::new(3), text)
    }

    fn mention(text: &str) -> MessageText {
        message(text).entities(vec![MessageEntityMention::new(0, 9).into()])
    }

    #[tokio::test]
    async fn guest_download_requires_own_mention_and_current_or_replied_link() {
        let mut observer = Observer::new("guest_message")
            .filter(GuestMention::new(1, "test_bot".into()))
            .register(Handler::new(|| async { Ok::<_, Infallible>(EventReturn::Finish) }).filter(text_contains_url_with_reply));
        let url = "https://media.example.test/video";
        for (message, expected_url) in [
            (mention(&format!("@test_bot {url}")), Some(url)),
            (mention("@test_bot").reply_to_message(message(url)), Some(url)),
            (mention("@test_bot"), None),
            (mention("@test_bot").reply_to_message(message("No media link")), None),
            (message("Thanks").reply_to_message(mention(&format!("@test_bot {url}"))), None),
            (message(url).reply_to_message(mention("@test_bot")), None),
            (message(url), None),
            (mention(&format!("@else_bot {url}")), None),
        ] {
            let request = Request {
                bot: Bot::new("1:synthetic"),
                update: Arc::new(Update::GuestMessage(UpdateGuestMessage::new(
                    1,
                    message.guest_query_id("synthetic-query"),
                ))),
                context: Context::default(),
                extensions: Extensions::default(),
            };
            let response = observer.trigger(request).await.unwrap();
            assert_eq!(
                matches!(response.propagate_result, PropagateEventResult::Handled(_)),
                expected_url.is_some()
            );
            assert_eq!(response.request.extensions.get::<Url>().map(Url::as_str), expected_url);
        }
    }

    #[test]
    fn mentions_use_current_entities_and_utf16_offsets() {
        let filter = GuestMention::new(1, "test_bot".into());
        let text = "😀 @TeSt_BoT";
        assert!(filter.matches(&message(text).entities(vec![MessageEntityMention::new(3, 9).into()]).into()));
        assert!(!filter.matches(&message("@test_bot").into()));
        for (offset, length) in [(-1, 9), (0, -1), (0, 100), (i64::MAX, i64::MAX)] {
            assert!(!filter.matches(
                &message("@test_bot")
                    .entities(vec![MessageEntityMention::new(offset, length).into()])
                    .into()
            ));
        }
        for (id, expected) in [(1, true), (4, false)] {
            let message = message("Bot")
                .entities(vec![MessageEntityTextMention::new(0, 3, User::new(id, true, "Bot")).into()])
                .into();
            assert_eq!(filter.matches(&message), expected);
        }
        let photo = MessagePhoto::new(
            2,
            1,
            ChatPrivate::new(3),
            vec![PhotoSize::new("synthetic-file", "synthetic-unique", 1, 1)],
        )
        .caption("@test_bot")
        .caption_entities(vec![MessageEntityMention::new(0, 9).into()]);
        assert!(filter.matches(&photo.into()));
    }
}
