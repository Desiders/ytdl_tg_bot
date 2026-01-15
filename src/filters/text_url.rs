use crate::config::BlacklistedConfig;

use froodi::async_impl::Container;
use std::{future::Future, str::FromStr};
use telers::{types::UpdateKind, Request};
use tracing::error;
use url::Url;

pub fn get_url_from_text(text: &str) -> Option<Url> {
    let words: Vec<&str> = text.split_whitespace().collect();
    for word in words {
        if let Ok(url) = Url::parse(word) {
            return Some(url);
        }
    }
    None
}

pub fn text_contains_url(request: &mut Request) -> impl Future<Output = bool> {
    let result = if let Some(text) = request.update.text() {
        let mut url_found = false;
        if let Some(url) = get_url_from_text(text) {
            if url.origin().is_tuple() {
                url_found = true;
                request.extensions.insert(url);
            }
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
                if url.origin().is_tuple() {
                    url_found = true;
                    request.extensions.insert(url);
                }
            }
            None => match request.update.kind() {
                UpdateKind::Message(message) | UpdateKind::EditedMessage(message) => {
                    if let Some(message) = message.reply_to_message() {
                        if let Some(text) = message.text() {
                            if let Some(url) = get_url_from_text(text) {
                                url_found = true;
                                request.extensions.insert(url);
                            }
                        }
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

pub fn url_is_blacklisted(request: &mut Request) -> impl Future<Output = bool> {
    let url_option = request.extensions.get::<Url>().cloned();
    let container_option = request.extensions.get::<Container>().cloned();
    async move {
        let Some(url) = url_option else {
            return false;
        };
        let Some(domain) = url.domain() else {
            return false;
        };
        let Some(container) = container_option else {
            return false;
        };
        match container.get::<BlacklistedConfig>().await {
            Ok(cfg) => cfg.domains.iter().map(String::as_str).collect::<Vec<_>>().contains(&domain),
            Err(err) => {
                error!(%err);
                false
            }
        }
    }
}

pub fn url_is_skippable_by_param(request: &mut Request) -> impl Future<Output = bool> {
    let mut result: bool = false;
    if let Some(url) = request.extensions.get::<Url>() {
        for (key, value) in url.query_pairs() {
            if ["yv2t", "yv2t_bot", "download"].contains(&&*key.to_lowercase()) && !bool::from_str(&value).unwrap_or(true) {
                result = true;
                break;
            }
        }
    }
    async move { result }
}
