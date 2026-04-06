use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::RwLock,
};
use url::Host;

#[derive(Debug, Clone)]
pub struct Cookie {
    pub cookie_id: Box<str>,
    pub domain: Box<str>,
    pub path: PathBuf,
}

pub struct Cookies {
    base_dir: PathBuf,
    cookies: RwLock<HashMap<String, Cookie>>,
}

impl Cookies {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            base_dir: path.as_ref().to_path_buf(),
            cookies: RwLock::new(HashMap::new()),
        }
    }

    pub fn clear_on_startup(&self) -> io::Result<()> {
        if self.base_dir.exists() {
            fs::remove_dir_all(&self.base_dir)?;
        }
        fs::create_dir_all(&self.base_dir)?;
        if let Ok(mut cookies) = self.cookies.write() {
            cookies.clear();
        }
        Ok(())
    }

    pub fn upsert_cookie(&self, cookie_id: &str, domain: &str, data: &str) -> io::Result<()> {
        fs::create_dir_all(&self.base_dir)?;
        let file_name = cookie_file_name(domain);
        let path = self.base_dir.join(file_name);
        fs::write(&path, data)?;

        let cookie = Cookie {
            cookie_id: cookie_id.into(),
            domain: normalize_domain(domain).into_boxed_str(),
            path,
        };
        if let Ok(mut cookies) = self.cookies.write() {
            cookies.insert(cookie_id.to_owned(), cookie);
        }
        Ok(())
    }

    pub fn remove_cookie(&self, cookie_id: &str) -> io::Result<()> {
        let removed = self.cookies.write().ok().and_then(|mut cookies| cookies.remove(cookie_id));
        if let Some(cookie) = removed {
            if cookie.path.exists() {
                fs::remove_file(cookie.path)?;
            }
        }
        Ok(())
    }

    pub fn get_path_by_host(&self, host: &Host<&str>) -> Option<PathBuf> {
        let host = normalize_domain(&host.to_string());
        let cookies = self.cookies.read().ok()?;
        cookies
            .values()
            .find(|cookie| cookie.domain.as_ref() == host)
            .map(|cookie| cookie.path.clone())
    }

    pub fn get_path_by_optional_host(&self, host: Option<&Host<&str>>) -> Option<PathBuf> {
        host.and_then(|h| self.get_path_by_host(h))
    }

    pub fn get_domains(&self) -> Vec<String> {
        let Some(cookies) = self.cookies.read().ok() else {
            return Vec::new();
        };
        let mut domains = cookies.values().map(|cookie| cookie.domain.to_string()).collect::<Vec<_>>();
        domains.sort();
        domains.dedup();
        domains
    }

    pub fn get_cookie_ids(&self) -> Vec<String> {
        let Some(cookies) = self.cookies.read().ok() else {
            return Vec::new();
        };
        cookies.values().map(|cookie| cookie.cookie_id.to_string()).collect()
    }
}

fn normalize_domain(domain: &str) -> String {
    domain.trim_start_matches("www.").to_owned()
}

fn cookie_file_name(domain: &str) -> String {
    format!("{}.txt", normalize_domain(domain))
}
