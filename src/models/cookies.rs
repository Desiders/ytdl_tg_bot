use std::path::PathBuf;
use url::Host;

#[derive(Debug, Clone)]
pub struct Cookie {
    pub host: Host,
    pub path: PathBuf,
}

#[derive(Debug, Default, Clone)]
pub struct Cookies {
    cookies: Vec<Cookie>,
}

impl Cookies {
    pub fn add_cookie(&mut self, host: Host, path: PathBuf) {
        self.cookies.push(Cookie { host, path });
    }

    pub fn get_path_by_host<'a>(&self, host: &Host<&'a str>) -> Option<&Cookie> {
        let host_str = host.to_string();
        let host_stripped = if let Some(stripped) = host_str.strip_prefix("www.") {
            stripped
        } else {
            host_str.as_str()
        };
        self.cookies.iter().find(|cookie| cookie.host.to_string() == host_stripped)
    }

    pub fn get_path_by_optional_host<'a>(&self, host: Option<&Host<&'a str>>) -> Option<&Cookie> {
        host.and_then(|h| self.get_path_by_host(h))
    }

    pub fn get_hosts(&self) -> Vec<&Host> {
        self.cookies.iter().map(|cookie| &cookie.host).collect()
    }
}
