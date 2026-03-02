use crate::config::BlacklistedConfig;

use froodi::async_impl::Container;
use psl::Psl;
use std::{future::Future, str::FromStr};
use telers::{types::Message, Request};
use tracing::error;
use url::{Host, Url};

pub fn get_url_from_text(text: &str) -> Option<Url> {
    let words: Vec<&str> = text.split_whitespace().collect();
    for word in words {
        if let Ok(url) = Url::parse(word) {
            return Some(url);
        }
    }
    None
}

pub fn get_host_from_text(text: &str) -> Option<Host> {
    let words: Vec<&str> = text.split_whitespace().collect();
    for word in words {
        if let Ok(url) = Url::parse(word) {
            if let Some(host) = url.host() {
                let host = host.to_owned();
                if let Some(suffix) = psl::List.suffix(host.to_string().as_bytes()) {
                    if suffix.is_known() && suffix.typ().unwrap() == psl::Type::Icann {
                        return Some(host);
                    }
                }
            }
        }
        if let Ok(host) = Host::parse(word) {
            if let Some(suffix) = psl::List.suffix(host.to_string().as_bytes()) {
                if suffix.is_known() && suffix.typ().unwrap() == psl::Type::Icann {
                    return Some(host);
                }
            }
        }
    }
    None
}

pub fn text_contains_url(request: &mut Request) -> impl Future<Output = bool> {
    let result = if let Some(text) = request.update.text().or(request.update.query()) {
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
    let result = if let Some(text) = request.update.text().or(request.update.query()) {
        let mut url_found = false;
        if let Some(url) = get_url_from_text(text) {
            if url.origin().is_tuple() {
                url_found = true;
                request.extensions.insert(url);
            }
        }
        if !url_found {
            if let Some(text) = request.update.reply_to_message().and_then(Message::text) {
                if let Some(url) = get_url_from_text(text) {
                    if url.origin().is_tuple() {
                        url_found = true;
                        request.extensions.insert(url);
                    }
                }
            }
        }
        url_found
    } else {
        false
    };
    async move { result }
}

#[allow(clippy::module_name_repetitions)]
pub fn text_contains_host_with_reply(request: &mut Request) -> impl Future<Output = bool> {
    let result = if let Some(text) = request.update.text().or(request.update.query()) {
        let mut host_found = false;
        if let Some(host) = get_host_from_text(text) {
            host_found = true;
            request.extensions.insert(host);
        }
        if !host_found {
            if let Some(text) = request.update.reply_to_message().and_then(Message::text) {
                if let Some(host) = get_host_from_text(text) {
                    host_found = true;
                    request.extensions.insert(host);
                }
            }
        }
        host_found
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
