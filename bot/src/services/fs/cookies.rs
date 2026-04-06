use std::{fs, io, path::Path};

use crate::entities::CookieRecord;

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

        let Some(file_name) = path.file_name().map(|name| name.to_string_lossy().to_string()) else {
            continue;
        };
        let Some(domain) = extract_domain_from_flat_name(&file_name) else {
            continue;
        };

        let cookie_id = file_name;
        cookies.push(CookieRecord {
            cookie_id,
            domain,
            path,
        });
    }

    Ok(cookies)
}

fn extract_domain_from_flat_name(file_name: &str) -> Option<String> {
    let stem = Path::new(file_name).file_stem()?.to_string_lossy().to_string();
    let (domain, _) = stem.rsplit_once('_')?;
    Some(domain.to_owned())
}
