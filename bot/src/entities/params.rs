use serde::{Deserialize, Serialize};
use std::{collections::HashMap, convert::Infallible, future::Future, ops::Range, str::FromStr as _};
use telers::{Extractor, Request};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Params(pub HashMap<String, String>);

impl Params {
    fn parse(text: &str) -> Self {
        Self::parse_with_span(text).map(|(params, _)| params).unwrap_or_default()
    }

    /// Finds the first `[...]` block whose content parses as params, returning the parsed params and
    /// the byte range of the block (so callers can strip it from the text).
    fn parse_with_span(text: &str) -> Option<(Self, Range<usize>)> {
        let mut search_start = 0;
        loop {
            let bracket_pos = search_start + text[search_start..].find('[')?;
            let start = bracket_pos + 1;
            let end = start + text[start..].find(']')?;

            if let Some(params) = Self::try_parse_content(&text[start..end]) {
                return Some((Params(params), bracket_pos..end + 1));
            }

            search_start = bracket_pos + 1;
        }
    }

    /// Returns `text` with the params block (the one [`parse`](Self::parse) reads) removed and
    /// surrounding whitespace collapsed — i.e. the clean text the user meant to send, e.g. for a
    /// search query. Brackets that aren't valid params are left untouched.
    pub fn strip_from(text: &str) -> String {
        let cleaned = match Self::parse_with_span(text) {
            Some((_, span)) => format!("{}{}", &text[..span.start], &text[span.end..]),
            None => text.to_owned(),
        };
        cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn try_parse_content(content: &str) -> Option<HashMap<String, String>> {
        let params: HashMap<String, String> = content
            .split(',')
            .filter_map(|param_str| {
                let (key, value) = param_str.trim().split_once('=')?;
                Some((key.trim().to_owned(), value.trim().to_owned()))
            })
            .collect();

        (!params.is_empty()).then_some(params)
    }

    pub fn get_bool(&self, key: &str) -> bool {
        self.0.get(key).is_some_and(|value| bool::from_str(value).unwrap_or(false))
    }
}

impl Extractor for Params {
    type Error = Infallible;

    fn extract(request: &Request) -> impl Future<Output = Result<Self, Self::Error>> + Send {
        let params = request
            .update
            .text()
            .or(request.update.query())
            .map(Params::parse)
            .unwrap_or_default();
        async move { Ok(params) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_params() {
        let params = Params::parse("[key=value]");
        assert_eq!(params.0.len(), 1);
        assert_eq!(params.0.get("key").map(AsRef::as_ref), Some("value"));
    }

    #[test]
    fn test_multiple_params() {
        let params = Params::parse("[key1=value1, key2=value2]");
        assert_eq!(params.0.len(), 2);
        assert_eq!(params.0.get("key1").map(AsRef::as_ref), Some("value1"));
        assert_eq!(params.0.get("key2").map(AsRef::as_ref), Some("value2"));
    }

    #[test]
    fn test_spaces_around_brackets() {
        let params = Params::parse("[  val    =3   , va=2]");
        assert_eq!(params.0.len(), 2);
        assert_eq!(params.0.get("val").map(AsRef::as_ref), Some("3"));
        assert_eq!(params.0.get("va").map(AsRef::as_ref), Some("2"));
    }

    #[test]
    fn test_empty_params() {
        let params = Params::parse("[]");
        assert_eq!(params.0.len(), 0);
    }

    #[test]
    fn test_params_in_middle_of_text() {
        let params = Params::parse("some text [key=value, foo=bar] and more text");
        assert_eq!(params.0.len(), 2);
        assert_eq!(params.0.get("key").map(AsRef::as_ref), Some("value"));
        assert_eq!(params.0.get("foo").map(AsRef::as_ref), Some("bar"));
    }

    #[test]
    fn test_params_at_end() {
        let params = Params::parse("prefix text [a=1, b=2]");
        assert_eq!(params.0.len(), 2);
        assert_eq!(params.0.get("a").map(AsRef::as_ref), Some("1"));
        assert_eq!(params.0.get("b").map(AsRef::as_ref), Some("2"));
    }

    #[test]
    fn test_params_at_start() {
        let params = Params::parse("[x=10] some suffix text");
        assert_eq!(params.0.len(), 1);
        assert_eq!(params.0.get("x").map(AsRef::as_ref), Some("10"));
    }

    #[test]
    fn test_no_brackets() {
        let params = Params::parse("just some text without brackets");
        assert_eq!(params.0.len(), 0);
    }

    #[test]
    fn test_only_opening_bracket() {
        let params = Params::parse("text [ no closing bracket");
        assert_eq!(params.0.len(), 0);
    }

    #[test]
    fn test_skips_invalid_bracket_finds_valid() {
        let params = Params::parse("[This is not params] [key=value, foo=bar]");
        assert_eq!(params.0.len(), 2);
        assert_eq!(params.0.get("key").map(AsRef::as_ref), Some("value"));
        assert_eq!(params.0.get("foo").map(AsRef::as_ref), Some("bar"));
    }

    #[test]
    fn test_multiple_invalid_then_valid() {
        let params = Params::parse("[no equals here] [also invalid] [a=1, b=2]");
        assert_eq!(params.0.len(), 2);
        assert_eq!(params.0.get("a").map(AsRef::as_ref), Some("1"));
        assert_eq!(params.0.get("b").map(AsRef::as_ref), Some("2"));
    }

    #[test]
    fn test_all_invalid_brackets() {
        let params = Params::parse("[not valid] [also not] [nope]");
        assert_eq!(params.0.len(), 0);
    }

    #[test]
    fn test_spaces_between_params() {
        let params = Params::parse("[key=value,  ,  , foo=bar]");
        assert_eq!(params.0.len(), 2);
        assert_eq!(params.0.get("key").map(AsRef::as_ref), Some("value"));
        assert_eq!(params.0.get("foo").map(AsRef::as_ref), Some("bar"));
    }

    #[test]
    fn test_url_with_items() {
        let params = Params::parse("https://www.youtube.com/playlist?list=... [items=:3:]");
        assert_eq!(params.0.len(), 1);
        assert_eq!(params.0.get("items").map(AsRef::as_ref), Some(":3:"));
    }

    #[test]
    fn test_get_bool_true() {
        let params = Params::parse("[overwrite=true]");
        assert!(params.get_bool("overwrite"));
    }

    #[test]
    fn test_get_bool_false() {
        let params = Params::parse("[overwrite=false]");
        assert!(!params.get_bool("overwrite"));
    }

    #[test]
    fn test_get_bool_invalid_defaults_to_true() {
        let params = Params::parse("[overwrite=force]");
        assert!(!params.get_bool("overwrite"));
    }

    #[test]
    fn test_get_bool_missing_defaults_to_false() {
        let params = Params::parse("[crop=1:00-2:00]");
        assert!(!params.get_bool("overwrite"));
    }

    #[test]
    fn strip_from_removes_trailing_params() {
        assert_eq!(Params::strip_from("test [lang=ru]"), "test");
    }

    #[test]
    fn strip_from_removes_leading_params() {
        assert_eq!(Params::strip_from("[lang=ru] test"), "test");
    }

    #[test]
    fn strip_from_removes_params_in_the_middle() {
        assert_eq!(Params::strip_from("the dark [lang=ru] knight"), "the dark knight");
    }

    #[test]
    fn strip_from_keeps_text_without_params() {
        assert_eq!(Params::strip_from("the dark knight"), "the dark knight");
    }

    #[test]
    fn strip_from_keeps_invalid_brackets() {
        assert_eq!(Params::strip_from("[not params] movie"), "[not params] movie");
    }

    #[test]
    fn strip_from_removes_only_the_first_valid_block() {
        assert_eq!(Params::strip_from("a [x=1] b [y=2]"), "a b [y=2]");
    }
}
