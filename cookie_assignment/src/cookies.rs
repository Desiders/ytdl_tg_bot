use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct CookieRecord {
    pub cookie_id: String,
    pub domain: String,
    pub path: PathBuf,
}

pub fn load_cookies_from_directory(root: &Path) -> Result<Vec<CookieRecord>, io::Error> {
    let mut cookies = Vec::new();

    if !root.exists() {
        return Ok(cookies);
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if !file_type.is_file() && !file_type.is_symlink() {
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "txt") {
            continue;
        }
        let Some(encoded_cookie_name) = path.file_name().map(|name| name.to_string_lossy().to_string()) else {
            continue;
        };
        let Some((domain, file_name)) = decode_cookie_entry_name(&encoded_cookie_name) else {
            continue;
        };

        cookies.push(CookieRecord {
            cookie_id: format!("{domain}/{file_name}"),
            domain,
            path,
        });
    }

    cookies.sort_by(|a, b| a.cookie_id.cmp(&b.cookie_id));
    Ok(cookies)
}

fn decode_cookie_entry_name(encoded_cookie_name: &str) -> Option<(String, String)> {
    let (domain, file_name) = encoded_cookie_name.split_once("__")?;
    if domain.is_empty() || file_name.is_empty() {
        return None;
    }
    Some((domain.to_owned(), file_name.to_owned()))
}
