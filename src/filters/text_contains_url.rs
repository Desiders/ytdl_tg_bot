use std::future::Future;
use telers::{
    types::{Update, UpdateKind},
    Bot, Context,
};
use tracing::{event, Level};
use url::Url;

fn get_url_from_text(text: &str) -> Option<Url> {
    for word in text.split_whitespace() {
        match Url::parse(word) {
            Ok(url) => {
                return Some(url);
            }
            Err(err) => {
                event!(Level::TRACE, %err, "Error while parsing url");

                continue;
            }
        };
    }

    None
}

pub fn text_contains_url(_bot: &Bot, update: &Update, context: &Context) -> impl Future<Output = bool> {
    let result = if let Some(text) = update.text() {
        let mut url_found = false;

        match get_url_from_text(text) {
            Some(url) => {
                url_found = true;

                context.insert("video_url", Box::new(url.as_str().to_owned().into_boxed_str()));
            }
            None => match update.kind() {
                UpdateKind::Message(message) | UpdateKind::EditedMessage(message) => {
                    if let Some(message) = message.reply_to_message() {
                        if let Some(text) = message.text() {
                            if let Some(url) = get_url_from_text(text) {
                                url_found = true;

                                context.insert("video_url", Box::new(url.as_str().to_owned().into_boxed_str()));
                            };
                        };
                    }
                }
                _ => {}
            },
        }

        url_found
    } else {
        false
    };

    async move { result }
}
