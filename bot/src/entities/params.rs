use std::{collections::HashMap, convert::Infallible, future::Future};
use telers::{Extractor, Request};

#[derive(Debug, Default, Clone)]
pub struct Params(pub HashMap<String, String>);

impl Params {
    fn parse(text: &str) -> Self {
        let mut search_start = 0;
        loop {
            let bracket_pos = match text[search_start..].find('[') {
                Some(pos) => search_start + pos,
                None => return Self::default(),
            };

            let start = bracket_pos + 1;

            let end = match text[start..].find(']') {
                Some(pos) => start + pos,
                None => return Self::default(),
            };

            let content = &text[start..end];

            if let Some(params) = Self::try_parse_content(content) {
                return Params(params);
            }

            search_start = bracket_pos + 1;
        }
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
}

impl Extractor for Params {
    type Error = Infallible;

    fn extract(request: &Request) -> impl Future<Output = Result<Self, Self::Error>> + Send {
        let params = request.update.text().map(Params::parse).unwrap_or_default();
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
        assert_eq!(params.0.get("key").map(|v| v.as_ref()), Some("value"));
    }

    #[test]
    fn test_multiple_params() {
        let params = Params::parse("[key1=value1, key2=value2]");
        assert_eq!(params.0.len(), 2);
        assert_eq!(params.0.get("key1").map(|v| v.as_ref()), Some("value1"));
        assert_eq!(params.0.get("key2").map(|v| v.as_ref()), Some("value2"));
    }

    #[test]
    fn test_spaces_around_brackets() {
        let params = Params::parse("[  val    =3   , va=2]");
        assert_eq!(params.0.len(), 2);
        assert_eq!(params.0.get("val").map(|v| v.as_ref()), Some("3"));
        assert_eq!(params.0.get("va").map(|v| v.as_ref()), Some("2"));
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
        assert_eq!(params.0.get("key").map(|v| v.as_ref()), Some("value"));
        assert_eq!(params.0.get("foo").map(|v| v.as_ref()), Some("bar"));
    }

    #[test]
    fn test_params_at_end() {
        let params = Params::parse("prefix text [a=1, b=2]");
        assert_eq!(params.0.len(), 2);
        assert_eq!(params.0.get("a").map(|v| v.as_ref()), Some("1"));
        assert_eq!(params.0.get("b").map(|v| v.as_ref()), Some("2"));
    }

    #[test]
    fn test_params_at_start() {
        let params = Params::parse("[x=10] some suffix text");
        assert_eq!(params.0.len(), 1);
        assert_eq!(params.0.get("x").map(|v| v.as_ref()), Some("10"));
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
        assert_eq!(params.0.get("key").map(|v| v.as_ref()), Some("value"));
        assert_eq!(params.0.get("foo").map(|v| v.as_ref()), Some("bar"));
    }

    #[test]
    fn test_multiple_invalid_then_valid() {
        let params = Params::parse("[no equals here] [also invalid] [a=1, b=2]");
        assert_eq!(params.0.len(), 2);
        assert_eq!(params.0.get("a").map(|v| v.as_ref()), Some("1"));
        assert_eq!(params.0.get("b").map(|v| v.as_ref()), Some("2"));
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
        assert_eq!(params.0.get("key").map(|v| v.as_ref()), Some("value"));
        assert_eq!(params.0.get("foo").map(|v| v.as_ref()), Some("bar"));
    }

    #[test]
    fn test_url_with_items() {
        let params = Params::parse("https://www.youtube.com/playlist?list=... [items=:3:]");
        assert_eq!(params.0.len(), 1);
        assert_eq!(params.0.get("items").map(|v| v.as_ref()), Some(":3:"));
    }
}
