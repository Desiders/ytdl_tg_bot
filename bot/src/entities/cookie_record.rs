use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CookieRecord {
    pub cookie_id: String,
    pub domain: String,
    pub path: PathBuf,
}
