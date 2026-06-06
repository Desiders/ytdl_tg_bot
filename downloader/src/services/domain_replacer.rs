use crate::config::{ReplaceDomainsConfig, ReplaceRule};

use regex::Regex;
use tracing::info;
use url::Url;

/// The media type a replacement is resolved for. Each type has its own rule set because the
/// downloaders differ (video/audio use yt-dlp, photo uses gallery-dl) and a proxy that works for
/// one extractor may not work for another.
#[derive(Clone, Copy, Debug)]
pub enum MediaKind {
    Video,
    Audio,
    Photo,
}

type Rules = Vec<(Regex, Box<str>)>;

/// Replaces a URL's host based on per-media-type regex rules.
///
/// The node uses this as a cookie-free fallback: when it has no cookie for a domain (or, for
/// video/audio, a cookied attempt fails), it reroutes the request through a proxy host (for
/// example Instagram through vxinstagram) that does not require the node to manage cookies itself.
pub struct DomainReplacer {
    video: Rules,
    audio: Rules,
    photo: Rules,
}

impl DomainReplacer {
    /// Compiles the replacement rules once, up front.
    ///
    /// # Panics
    ///
    /// Panics if a `from` pattern is not a valid regex.
    #[must_use]
    pub fn new(config: &ReplaceDomainsConfig) -> Self {
        Self {
            video: compile_rules(&config.video),
            audio: compile_rules(&config.audio),
            photo: compile_rules(&config.photo),
        }
    }

    /// Returns the URL with its host replaced by the first matching rule for `kind`, or `None` when
    /// no rule matches (the caller then keeps the original URL unchanged).
    #[must_use]
    pub fn replace_url(&self, url: &Url, kind: MediaKind) -> Option<Url> {
        let rules = match kind {
            MediaKind::Video => &self.video,
            MediaKind::Audio => &self.audio,
            MediaKind::Photo => &self.photo,
        };

        let domain = url.domain()?;
        for (re, to) in rules {
            if re.is_match(domain) {
                let host = re.replace(domain, &**to);
                let mut replaced = url.clone();
                replaced.set_host(Some(&host)).expect("Invalid host");
                info!(from = domain, to = %host, ?kind, "Replace domain");
                return Some(replaced);
            }
        }
        None
    }
}

fn compile_rules(rules: &[ReplaceRule]) -> Rules {
    rules
        .iter()
        .map(|ReplaceRule { from, to }| (Regex::new(from).expect("Invalid `from` pattern"), to.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{DomainReplacer, MediaKind};
    use crate::config::{ReplaceDomainsConfig, ReplaceRule};
    use url::Url;

    fn instagram_rule() -> ReplaceRule {
        ReplaceRule {
            from: r"^(?:[a-z0-9-]+\.)?instagram\.(tv|net|com|org)|instagr\.am".into(),
            to: "vxinstagram.com".into(),
        }
    }

    #[test]
    fn replaces_for_configured_media_type_and_keeps_path_and_query() {
        let replacer = DomainReplacer::new(&ReplaceDomainsConfig {
            video: vec![instagram_rule()],
            ..Default::default()
        });

        let replaced = replacer
            .replace_url(&Url::parse("https://www.instagram.com/reel/xyz?hl=en").unwrap(), MediaKind::Video)
            .expect("expected replacement");

        assert_eq!(replaced.host_str(), Some("vxinstagram.com"));
        assert_eq!(replaced.path(), "/reel/xyz");
        assert_eq!(replaced.query(), Some("hl=en"));
    }

    #[test]
    fn does_not_replace_for_media_type_without_rules() {
        let replacer = DomainReplacer::new(&ReplaceDomainsConfig {
            video: vec![instagram_rule()],
            ..Default::default()
        });

        // Photo has no rules, so the same URL is left unchanged even though video would replace it.
        assert!(replacer
            .replace_url(&Url::parse("https://instagram.com/p/abc").unwrap(), MediaKind::Photo)
            .is_none());
    }

    #[test]
    fn returns_none_when_no_rule_matches() {
        let replacer = DomainReplacer::new(&ReplaceDomainsConfig {
            video: vec![instagram_rule()],
            ..Default::default()
        });

        assert!(replacer
            .replace_url(&Url::parse("https://youtube.com/watch?v=abc").unwrap(), MediaKind::Video)
            .is_none());
    }

    #[test]
    fn applies_first_matching_rule() {
        let replacer = DomainReplacer::new(&ReplaceDomainsConfig {
            photo: vec![
                ReplaceRule {
                    from: "example.com".into(),
                    to: "first.example".into(),
                },
                ReplaceRule {
                    from: "example.com".into(),
                    to: "second.example".into(),
                },
            ],
            ..Default::default()
        });

        let replaced = replacer
            .replace_url(&Url::parse("https://example.com/path").unwrap(), MediaKind::Photo)
            .expect("expected replacement");

        assert_eq!(replaced.host_str(), Some("first.example"));
    }
}
