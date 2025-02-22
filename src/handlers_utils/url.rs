use std::collections::HashMap;

use url::Url;

#[derive(Clone)]
pub struct UrlWithParams {
    pub url: Url,
    pub params: HashMap<Box<str>, Box<str>>,
}
