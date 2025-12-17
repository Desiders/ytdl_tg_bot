use crate::{config::BlacklistedConfig, entities::UrlWithParams};

use froodi::async_impl::Container;
use std::{collections::HashMap, future::Future, str::FromStr};
use telers::{types::UpdateKind, Request};
use tracing::error;
use url::Url;

fn parse_params(param_block: &str) -> Vec<(Box<str>, Box<str>)> {
    let inner = param_block.trim();
    // Remove the surrounding brackets if they exist.
    let inner = inner.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(inner);
    let params = inner
        .split(',')
        .filter_map(|param| {
            let trimmed = param.trim();
            if trimmed.is_empty() {
                None
            } else {
                let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
                if parts.len() == 2 {
                    let key = parts[0].trim();
                    let value = parts[1].trim();
                    if key.is_empty() || value.is_empty() {
                        None
                    } else {
                        Some((key.to_owned().into_boxed_str(), value.to_owned().into_boxed_str()))
                    }
                } else {
                    None
                }
            }
        })
        .collect::<Vec<_>>();
    params
}

/// Extracts a URL and its following parameters from the given text.
///
/// Splits the text by whitespace, finds the first valid URL,
/// and, if the subsequent word is a parameter block (enclosed in [ ]), parses it.
/// Parameters must be in "key=value" format, separated by commas.
pub fn get_url_with_params_from_text(text: &str) -> Option<UrlWithParams> {
    let words: Vec<&str> = text.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        if let Ok(url) = Url::parse(word) {
            let params = if let Some(next_word) = words.get(i + 1) {
                if next_word.starts_with('[') && next_word.ends_with(']') {
                    parse_params(next_word)
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            return Some(UrlWithParams {
                url,
                params: HashMap::from_iter(params),
            });
        }
    }
    None
}

pub fn text_contains_url(request: &mut Request) -> impl Future<Output = bool> {
    let result = if let Some(text) = request.update.text() {
        let mut url_found = false;

        if let Some(url_with_params) = get_url_with_params_from_text(text) {
            if url_with_params.url.origin().is_tuple() {
                url_found = true;
                request.extensions.insert(url_with_params);
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

        match get_url_with_params_from_text(text) {
            Some(url_with_params) => {
                if url_with_params.url.origin().is_tuple() {
                    url_found = true;
                    request.extensions.insert(url_with_params);
                }
            }
            None => match request.update.kind() {
                UpdateKind::Message(message) | UpdateKind::EditedMessage(message) => {
                    if let Some(message) = message.reply_to_message() {
                        if let Some(text) = message.text() {
                            if let Some(url_with_params) = get_url_with_params_from_text(text) {
                                url_found = true;
                                request.extensions.insert(url_with_params);
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
    let url_with_params_option = request.extensions.get::<UrlWithParams>().cloned();
    let container_option = request.extensions.get::<Container>().cloned();
    async move {
        let Some(UrlWithParams { url, .. }) = url_with_params_option else {
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
                return false;
            }
        }
    }
}

pub fn url_is_skippable_by_param(request: &mut Request) -> impl Future<Output = bool> {
    let mut result: bool = false;
    if let Some(UrlWithParams { url, .. }) = request.extensions.get::<UrlWithParams>() {
        for (key, value) in url.query_pairs() {
            if ["yv2t", "yv2t_bot", "download"].contains(&&*key.to_lowercase()) && !bool::from_str(&value).unwrap_or(true) {
                result = true;
                break;
            }
        }
    }
    async move { result }
}
