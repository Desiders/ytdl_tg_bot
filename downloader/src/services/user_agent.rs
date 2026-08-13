use crate::config::UserAgentRule;

use tracing::debug;
use url::Url;

pub struct UserAgentResolver {
    rules: Vec<(Vec<Box<str>>, Box<str>)>,
}

impl UserAgentResolver {
    #[must_use]
    pub fn new(rules: &[UserAgentRule]) -> Self {
        Self {
            rules: rules
                .iter()
                .map(|UserAgentRule { domains, user_agent }| {
                    let domains = domains
                        .iter()
                        .map(|domain| domain.trim().trim_start_matches('.').to_ascii_lowercase().into())
                        .collect();
                    (domains, user_agent.clone())
                })
                .collect(),
        }
    }

    #[must_use]
    pub fn resolve(&self, url: &Url) -> Option<&str> {
        let domain = url.domain()?;
        for (domains, user_agent) in &self.rules {
            if domains.iter().any(|configured| {
                domain
                    .strip_suffix(&**configured)
                    .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('.'))
            }) {
                debug!(domain, %user_agent, "Using custom user agent");
                return Some(user_agent);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::UserAgentResolver;
    use crate::config::UserAgentRule;
    use url::Url;

    fn resolver() -> UserAgentResolver {
        UserAgentResolver::new(&[UserAgentRule {
            domains: vec!["tiktok.com".into()],
            user_agent: "TikTokUA/1.0".into(),
        }])
    }

    fn resolve<'a>(resolver: &'a UserAgentResolver, url: &str) -> Option<&'a str> {
        resolver.resolve(&Url::parse(url).unwrap())
    }

    #[test]
    fn resolves_for_domain_and_its_subdomains() {
        let resolver = resolver();

        for url in [
            "https://tiktok.com/@user/photo/1",
            "https://www.tiktok.com/@user/photo/1",
            "https://vt.tiktok.com/ZSQa9QDCe/",
        ] {
            assert_eq!(resolve(&resolver, url), Some("TikTokUA/1.0"), "{url}");
        }
    }

    #[test]
    fn does_not_match_domain_with_the_same_suffix_text() {
        assert!(resolve(&resolver(), "https://nottiktok.com/watch").is_none());
    }

    #[test]
    fn matching_is_case_insensitive_and_tolerates_leading_dot() {
        let resolver = UserAgentResolver::new(&[UserAgentRule {
            domains: vec![".TikTok.com".into()],
            user_agent: "ua".into(),
        }]);

        assert_eq!(resolve(&resolver, "https://VT.TikTok.COM/x"), Some("ua"));
    }

    #[test]
    fn returns_none_when_no_rule_matches() {
        assert!(resolve(&resolver(), "https://www.youtube.com/watch?v=abc").is_none());
    }

    #[test]
    fn returns_none_without_rules() {
        let resolver = UserAgentResolver::new(&[]);

        assert!(resolve(&resolver, "https://www.tiktok.com/@user/video/1").is_none());
    }

    #[test]
    fn applies_first_matching_rule() {
        let resolver = UserAgentResolver::new(&[
            UserAgentRule {
                domains: vec!["example.com".into()],
                user_agent: "first".into(),
            },
            UserAgentRule {
                domains: vec!["example.com".into()],
                user_agent: "second".into(),
            },
        ]);

        assert_eq!(resolve(&resolver, "https://example.com/path"), Some("first"));
    }
}
