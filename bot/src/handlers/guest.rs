use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::{Message, MessageEntity, UpdateGuestMessage},
};
use url::Url;

use crate::{
    filters::get_url_from_text,
    interactors::{
        guest::{EnqueueGuestDownload, GuestInput},
        Interactor as _,
    },
    locale::Locale,
    services::messenger::MessengerPort,
    value_objects::MediaType,
};

pub async fn download<Messenger: MessengerPort>(
    update: UpdateGuestMessage,
    Inject(interactor): Inject<EnqueueGuestDownload<Messenger>>,
) -> HandlerResult {
    let message = &update.guest_message;
    let Some(query_id) = message.guest_query_id().filter(|id| !id.is_empty()) else {
        return Ok(EventReturn::Finish);
    };
    let url = message_url(message).or_else(|| message.reply_to_message().and_then(message_url));
    let locale = Locale::from_code(message.from().and_then(|user| user.language_code.as_deref()));
    let media_type = requested_media_type(message.text().or(message.caption()).unwrap_or_default());
    interactor
        .execute(GuestInput {
            query_id,
            url,
            locale,
            media_type,
        })
        .await?;
    Ok(EventReturn::Finish)
}

fn message_url(message: &Message) -> Option<Url> {
    let text = message.text().or(message.caption());
    let entity_url = || {
        message
            .entities()
            .or(message.caption_entities())?
            .iter()
            .find_map(|entity| match entity {
                MessageEntity::TextLink(link) => Url::parse(&link.url).ok(),
                MessageEntity::Url(link) => {
                    let offset = usize::try_from(link.offset).ok()?;
                    let length = usize::try_from(link.length).ok()?;
                    let units: Vec<_> = text?.encode_utf16().skip(offset).take(length).collect();
                    if units.len() != length {
                        return None;
                    }
                    Url::parse(&String::from_utf16(&units).ok()?).ok()
                }
                _ => None,
            })
    };
    entity_url().or_else(|| text.and_then(get_url_from_text))
}

fn requested_media_type(text: &str) -> Option<MediaType> {
    text.split_whitespace().find_map(|word| match word {
        "/vd" | "/video" | "/video_download" => Some(MediaType::Video),
        "/ad" | "/audio" | "/audio_download" => Some(MediaType::Audio),
        "/pd" | "/photo" | "/photo_download" => Some(MediaType::Photo),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(fields: &serde_json::Value) -> Message {
        let mut value = json!({"message_id": 1, "date": 1, "chat": {"id": 7, "type": "private", "first_name": "Synthetic"}});
        value.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn guest_url_entities_use_utf16_offsets() {
        let text = "🙂 https://example.test/video.";
        let message = message(&json!({
            "text": text,
            "entities": [{"type": "url", "offset": 3, "length": 26}]
        }));
        assert_eq!(message_url(&message).unwrap().as_str(), "https://example.test/video");
    }

    #[test]
    fn guest_reads_hidden_links_in_captions() {
        let message = message(&json!({
            "photo": [{"file_id": "synthetic", "file_unique_id": "synthetic", "width": 1, "height": 1}],
            "caption": "Media",
            "caption_entities": [{"type": "text_link", "offset": 0, "length": 5, "url": "https://example.test/media"}]
        }));
        assert_eq!(message_url(&message).unwrap().as_str(), "https://example.test/media");
    }

    #[test]
    fn guest_reply_can_supply_the_url() {
        let message = message(&json!({"text": "@synthetic_bot", "reply_to_message": {
            "message_id": 2, "date": 1, "chat": {"id": 7, "type": "private", "first_name": "Synthetic"},
            "text": "https://example.test/media"
        }}));
        let url = message_url(&message).or_else(|| message.reply_to_message().and_then(message_url));
        assert_eq!(url.unwrap().as_str(), "https://example.test/media");
    }

    #[test]
    fn guest_commands_work_after_the_mention() {
        assert!(matches!(
            requested_media_type("@synthetic_bot /ad https://example.test/media"),
            Some(MediaType::Audio)
        ));
        assert!(requested_media_type("@synthetic_bot https://example.test/media").is_none());
    }
}
