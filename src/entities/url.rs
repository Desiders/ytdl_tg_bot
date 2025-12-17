use std::collections::HashMap;
use url::Url;

#[derive(Clone)]
#[allow(clippy::module_name_repetitions)]
pub struct UrlWithParams {
    pub url: Url,
    pub params: HashMap<Box<str>, Box<str>>,
}
