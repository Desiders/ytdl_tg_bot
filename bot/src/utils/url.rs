use clear_urls::UrlCleaner as ClearUrls;
use serde_json::{json, Map};
use thiserror::Error;
use url::Url;

use crate::config::{TrackingParamsConfig, TrackingParamsRule};

#[derive(Debug, Error)]
pub enum ErrorKind {
    #[error("Invalid URL format")]
    InvalidUrl,

    #[error("No video ID found")]
    NoVideoIdFound,
}

pub struct UrlCleaner {
    inner: ClearUrls,
    config_rules: Option<ClearUrls>,
    extra_params: Box<[Box<str>]>,
}

impl UrlCleaner {
    pub fn from_embedded_rules(cfg: &TrackingParamsConfig) -> Result<Self, clear_urls::Error> {
        let config_rules = if cfg.rules.is_empty() {
            None
        } else {
            Some(ClearUrls::from_rules_json(&rules_json(&cfg.rules))?)
        };
        Ok(Self {
            inner: ClearUrls::from_embedded_rules()?,
            config_rules,
            extra_params: cfg.params.clone().into(),
        })
    }

    #[must_use]
    pub fn clean(&self, url: &Url) -> Option<Url> {
        let mut result = self.inner.clean(url.as_str());
        if let Some(config_rules) = &self.config_rules {
            let config_result = config_rules.clean(&result.url);
            result.changed |= config_result.changed;
            result.url = config_result.url;
        }
        let mut cleaned = if result.changed {
            Url::parse(&result.url).ok()?
        } else {
            url.clone()
        };

        let extra_removed = self.remove_extra_params(&mut cleaned);
        (result.changed || extra_removed).then_some(cleaned)
    }

    fn remove_extra_params(&self, url: &mut Url) -> bool {
        if self.extra_params.is_empty() || url.query().is_none() {
            return false;
        }

        let params = url
            .query_pairs()
            .filter(|(key, _)| self.extra_params.iter().all(|param| **param != **key))
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Box<[_]>>();
        if params.len() == url.query_pairs().count() {
            return false;
        }

        if params.is_empty() {
            url.set_query(None);
        } else {
            url.query_pairs_mut().clear().extend_pairs(params);
        }
        true
    }
}

/// Builds a `ClearURLs` rules document, so `clear-urls` matches `params` case-insensitively against whole parameter names.
fn rules_json(rules: &[TrackingParamsRule]) -> String {
    let providers = rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            (
                format!("tracking_params.rules[{index}]"),
                json!({ "urlPattern": rule.url_pattern, "rules": rule.params }),
            )
        })
        .collect::<Map<_, _>>();
    json!({ "providers": providers }).to_string()
}

const VIDEO_ID_LENGTH: usize = 11;

pub fn get_video_id(input: &str) -> Result<String, ErrorKind> {
    let url = Url::parse(input).map_err(|_| ErrorKind::InvalidUrl)?;
    let host = url.host_str().ok_or(ErrorKind::InvalidUrl)?;

    if host.ends_with("youtube.com") {
        if let Some(query_pairs) = url.query_pairs().find(|(key, _)| key == "v") {
            let id = query_pairs.1.to_string();
            if id.len() == VIDEO_ID_LENGTH {
                return Ok(id);
            }
        }

        let path_segments: Vec<&str> = url.path_segments().map(Iterator::collect).unwrap_or_default();
        if path_segments.len() > 1 {
            match path_segments[0] {
                "embed" | "v" | "shorts" if path_segments[1].len() == VIDEO_ID_LENGTH => return Ok(path_segments[1].to_string()),
                "clip" => return Ok(path_segments[1].to_string()),
                _ => {}
            }
        }
    } else if host == "youtu.be" {
        let path_segments: Vec<&str> = url.path_segments().map(Iterator::collect).unwrap_or_default();
        if let Some(id) = path_segments.first() {
            if id.len() == VIDEO_ID_LENGTH {
                return Ok((*id).to_owned());
            }
        }
    }

    Err(ErrorKind::NoVideoIdFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_cleaner() -> UrlCleaner {
        let cfg = crate::config::parse_from_fs(concat!(env!("CARGO_MANIFEST_DIR"), "/../configs/config.example.toml")).unwrap();
        UrlCleaner::from_embedded_rules(&cfg.tracking_params).unwrap()
    }

    #[test]
    fn example_rules_strip_instagram_share_params() {
        let cleaner = example_cleaner();
        for (url, expected) in [
            (
                "https://www.instagram.com/reel/AAAAAAAAAAA/?obrf=c3ludGhldGlj",
                "https://www.instagram.com/reel/AAAAAAAAAAA/",
            ),
            (
                "https://www.instagram.com/synthetic_user/reel/AAAAAAAAAAA/?vrfl=c3ludGhldGlj==",
                "https://www.instagram.com/synthetic_user/reel/AAAAAAAAAAA/",
            ),
            (
                "https://instagr.am/p/AAAAAAAAAAA/?xtok=c3ludGhldGlj",
                "https://instagr.am/p/AAAAAAAAAAA/",
            ),
            (
                "https://instagram.com/p/AAAAAAAAAAA/?utm_source=synthetic&obrf=c3ludGhldGlj&img_index=2&igsh=synthetic",
                "https://instagram.com/p/AAAAAAAAAAA/?img_index=2",
            ),
        ] {
            assert_eq!(cleaner.clean(&Url::parse(url).unwrap()).unwrap().as_str(), expected);
        }
    }

    #[test]
    fn example_rules_keep_other_params() {
        let cleaner = example_cleaner();
        for url in [
            "https://example.test/watch?obrf=synthetic",
            "https://instagram.com@example.test/reel/AAAAAAAAAAA/?obrf=synthetic",
            "https://www.instagram.com/p/AAAAAAAAAAA/?img_index=2",
            "https://www.instagram.com/s/c3ludGhldGlj?story_media_id=1",
        ] {
            assert_eq!(cleaner.clean(&Url::parse(url).unwrap()), None, "{url}");
        }
    }

    #[test]
    fn config_rule_with_invalid_regex_is_rejected() {
        let cfg = TrackingParamsConfig {
            params: vec![],
            rules: vec![TrackingParamsRule {
                url_pattern: "example\\.test".into(),
                params: vec!["(".into()],
            }],
        };
        assert!(matches!(
            UrlCleaner::from_embedded_rules(&cfg),
            Err(clear_urls::Error::InvalidRegex { .. })
        ));
    }

    #[test]
    fn test_valid_url_standard() {
        assert_eq!(get_video_id("https://www.youtube.com/watch?v=abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_with_params() {
        assert_eq!(
            get_video_id("https://www.youtube.com/watch?v=abcdefghijk&si=someparam").unwrap(),
            "abcdefghijk"
        );
    }

    #[test]
    fn test_valid_url_youtu_be_with_params() {
        assert_eq!(get_video_id("https://youtu.be/abcdefghijk?si=someparam").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_mobile() {
        assert_eq!(get_video_id("https://m.youtube.com/watch?v=abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_embed() {
        assert_eq!(get_video_id("https://www.youtube.com/embed/abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_shorts() {
        assert_eq!(get_video_id("https://www.youtube.com/shorts/abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_clips() {
        assert_eq!(
            get_video_id("https://youtube.com/clip/UgkxAunuGm-TAKLTuefe7EpYiyFPLfTWR28D?si=pC4WroQ7YCmPm3wO").unwrap(),
            "UgkxAunuGm-TAKLTuefe7EpYiyFPLfTWR28D"
        );
    }

    #[test]
    fn test_valid_url_youtu_be() {
        assert_eq!(get_video_id("https://youtu.be/abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_invalid_url_example() {
        assert!(get_video_id("https://www.example.com/watch?v=abcdefghijk").is_err());
    }

    #[test]
    fn test_invalid_url_non_video_path() {
        assert!(get_video_id("https://youtube.com/someotherpath").is_err());
    }

    #[test]
    fn test_invalid_url_short_id() {
        assert!(get_video_id("https://youtu.be/shortvideo").is_err());
    }

    #[test]
    fn test_invalid_url_not_a_url() {
        assert!(get_video_id("not a url").is_err());
    }

    #[test]
    fn test_invalid_url_homepage() {
        assert!(get_video_id("https://www.youtube.com/").is_err());
    }
}
