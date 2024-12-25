use std::future::Future;
use telers::{types::UpdateKind, Request};
use url::Url;

fn get_url_from_text(text: &str) -> Option<Url> {
    for word in text.split_whitespace() {
        match Url::parse(word) {
            Ok(url) => {
                return Some(url);
            }
            Err(_) => {
                continue;
            }
        };
    }

    None
}

pub fn text_contains_url(request: &mut Request) -> impl Future<Output = bool> {
    let result = if let Some(text) = request.update.text() {
        let mut url_found = false;

        if let Some(url) = get_url_from_text(text) {
            url_found = true;

            request.context.insert("video_url", url.as_str().to_owned().into_boxed_str());
        }

        url_found
    } else {
        false
    };

    async move { result }
}

#[allow(clippy::module_name_repetitions)]
pub fn text_contains_url_with_reply(request: &mut Request) -> impl Future<Output = bool> {
    let result = if let Some(text) = request.update.text() {
        let mut url_found = false;

        match get_url_from_text(text) {
            Some(url) => {
                url_found = true;

                request.context.insert("video_url", url.as_str().to_owned().into_boxed_str());
            }
            None => match request.update.kind() {
                UpdateKind::Message(message) | UpdateKind::EditedMessage(message) => {
                    if let Some(message) = message.reply_to_message() {
                        if let Some(text) = message.text() {
                            if let Some(url) = get_url_from_text(text) {
                                url_found = true;

                                request.context.insert("video_url", url.as_str().to_owned().into_boxed_str());
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
