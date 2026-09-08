use crate::config::BlacklistedConfig;

use froodi::async_impl::Container;
use psl::Psl;
use std::{convert::Infallible, future::Future, str::FromStr};
use telers::{types::Message, FilterResult, Request};
use tracing::{error, info};
use url::{Host, Url};

pub fn get_url_from_text(text: &str) -> Option<Url> {
    let words: Vec<&str> = text.split_whitespace().collect();
    for word in words {
        if let Ok(url) = Url::parse(word) {
            if url.origin().is_tuple() && url.domain().is_some() {
                return Some(url);
            }
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

pub fn text_contains_url(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
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
    async move { Ok(result) }
}

#[allow(clippy::module_name_repetitions)]
pub fn text_contains_url_with_reply(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
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
    async move { Ok(result) }
}

#[allow(clippy::module_name_repetitions)]
pub fn text_contains_host_with_reply(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
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
    async move { Ok(result) }
}

pub fn url_is_blacklisted(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let url_option = request.extensions.get::<Url>().cloned();
    let container_option = request.extensions.get::<Container>().cloned();
    let chat_id = request.update.chat().map(telers::types::Chat::id);
    async move {
        let Some(url) = url_option else {
            return Ok(false);
        };
        let Some(domain) = url.domain() else {
            return Ok(false);
        };
        let Some(container) = container_option else {
            return Ok(false);
        };
        Ok(match container.get::<BlacklistedConfig>().await {
            Ok(cfg) => {
                let blacklisted = cfg.domains.iter().any(|blacklisted| blacklisted == domain);
                if blacklisted {
                    info!(?chat_id, domain, "Skipping blacklisted domain");
                }
                blacklisted
            }
            Err(err) => {
                error!(%err);
                false
            }
        })
    }
}

pub fn url_is_skippable_by_param(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let mut result: bool = false;
    if let Some(url) = request.extensions.get::<Url>() {
        for (key, value) in url.query_pairs() {
            if ["yv2t", "yv2t_bot", "download"].contains(&&*key.to_lowercase()) && !bool::from_str(&value).unwrap_or(true) {
                result = true;
                break;
            }
        }
    }
    async move { Ok(result) }
}

#[cfg(test)]
mod tests {
    use super::{get_host_from_text, get_url_from_text};

    #[test]
    fn skips_non_network_scheme_and_returns_next_url() {
        let url = get_url_from_text("text: https://example.com/watch?v=1").expect("expected URL");

        assert_eq!(url.as_str(), "https://example.com/watch?v=1");
    }

    #[test]
    fn skips_ip_literal_hosts() {
        for text in [
            "https://127.0.0.1",
            "https://127.0.0.1:8080/x",
            "http://192.168.1.10/clip.mp4",
            "https://10",
            "https://2130706433",
            "https://0x7f.1",
            "https://[::1]/x",
        ] {
            assert!(get_url_from_text(text).is_none(), "{text}");
        }

        let url = get_url_from_text("https://10 https://example.com/x").expect("expected URL");

        assert_eq!(url.as_str(), "https://example.com/x");
    }

    #[test]
    fn skips_non_network_schemes_and_bare_hosts() {
        for text in [
            "mailto:someone@example.com",
            "file:///tmp/video.mp4",
            "javascript:alert(1)",
            "data:text/plain,hi",
            "example.com/watch?v=1",
            "www.example.com",
            "",
        ] {
            assert!(get_url_from_text(text).is_none(), "{text}");
        }
    }

    #[test]
    fn takes_first_network_url_among_words() {
        let url = get_url_from_text("look at this: https://one.example/a and https://two.example/b").expect("expected URL");

        assert_eq!(url.as_str(), "https://one.example/a");
    }

    #[test]
    fn keeps_query_fragment_port_and_userinfo() {
        let url = get_url_from_text("https://user:pw@example.com:8443/v?id=1&x=2#t=3").expect("expected URL");

        assert_eq!(url.as_str(), "https://user:pw@example.com:8443/v?id=1&x=2#t=3");
        assert_eq!(url.domain(), Some("example.com"));
    }

    #[test]
    fn normalizes_scheme_and_host_case() {
        let url = get_url_from_text("HTTPS://WWW.EXAMPLE.COM/Path").expect("expected URL");

        assert_eq!(url.as_str(), "https://www.example.com/Path");
    }

    #[test]
    fn accepts_other_network_schemes() {
        assert_eq!(get_url_from_text("http://example.com/a").unwrap().as_str(), "http://example.com/a");
        assert_eq!(get_url_from_text("ftp://example.com/a").unwrap().scheme(), "ftp");
    }

    #[test]
    fn host_from_text_requires_known_public_suffix() {
        assert_eq!(
            get_host_from_text("https://sub.example.co.uk/x").unwrap().to_string(),
            "sub.example.co.uk"
        );
        assert_eq!(get_host_from_text("block example.com please").unwrap().to_string(), "example.com");
        for text in [
            "localhost",
            "https://localhost/x",
            "127.0.0.1",
            "https://127.0.0.1/x",
            "intranet.localdomain",
            "",
        ] {
            assert!(get_host_from_text(text).is_none(), "{text}");
        }
    }

    #[test]
    fn returns_none_when_only_non_network_scheme_exists() {
        let url = get_url_from_text("text:");

        assert!(url.is_none());
    }
}
