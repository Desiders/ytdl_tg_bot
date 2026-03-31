#![allow(clippy::module_name_repetitions)]

use anyhow::Context;
use serde::Deserialize;
use std::{
    env::{self, VarError},
    fs, io,
    path::Path,
};
use thiserror::Error;
use tonic::transport::{Certificate, Identity, ServerTlsConfig};

#[derive(Deserialize, Clone, Debug)]
pub struct ServerConfig {
    pub address: Box<str>,
    pub max_concurrent: u32,
}

#[derive(Deserialize, Clone, Debug)]
pub struct AuthConfig {
    pub tokens: Vec<Box<str>>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct TlsConfig {
    pub ca_cert_path: Box<str>,
    pub cert_path: Box<str>,
    pub key_path: Box<str>,
}

#[derive(Default, Deserialize, Clone, Debug)]
pub struct YtDlpConfig {
    #[serde(default)]
    pub command: Vec<Box<str>>,
    #[serde(default)]
    pub executable_path: Option<Box<str>>,
    #[serde(default)]
    pub plugin_dirs: Vec<Box<str>>,
    pub cookies_path: Box<str>,
    pub max_file_size: u64,
}

impl YtDlpConfig {
    #[must_use]
    pub fn command_parts(&self) -> (&str, Vec<&str>) {
        if let Some((program, args)) = self.command.split_first() {
            return (program.as_ref(), args.iter().map(AsRef::as_ref).collect());
        }

        if let Some(executable_path) = self.executable_path.as_deref() {
            return (executable_path, Vec::new());
        }

        ("python3", vec!["-m", "yt_dlp"])
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct YtPotProviderConfig {
    pub url: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LoggingConfig {
    pub dirs: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub tls: TlsConfig,
    pub yt_dlp: YtDlpConfig,
    pub yt_pot_provider: YtPotProviderConfig,
    pub logging: LoggingConfig,
}

impl Config {
    pub fn load_server_tls_cfg(&self) -> anyhow::Result<ServerTlsConfig> {
        let config = &self.tls;
        let ca_cert_pem =
            fs::read(&*config.ca_cert_path).with_context(|| format!("Failed to read downloader CA certificate {}", config.ca_cert_path))?;
        let cert_pem = fs::read(&*config.cert_path).with_context(|| format!("Failed to read node certificate {}", config.cert_path))?;
        let key_pem = fs::read(&*config.key_path).with_context(|| format!("Failed to read node key {}", config.key_path))?;

        let tls_cfg = ServerTlsConfig::new()
            .client_ca_root(Certificate::from_pem(ca_cert_pem))
            .identity(Identity::from_pem(cert_pem, key_pem));
        Ok(tls_cfg)
    }
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error(transparent)]
    IO(#[from] io::Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
}

#[must_use]
pub fn get_path() -> Box<str> {
    let path = match env::var("DOWNLOADER_CONFIG_PATH") {
        Ok(val) => val,
        Err(VarError::NotPresent) => String::from("configs/downloader.toml"),
        Err(VarError::NotUnicode(_)) => {
            panic!("`DOWNLOADER_CONFIG_PATH` env variable is not a valid UTF-8 string!");
        }
    };

    path.into_boxed_str()
}

#[allow(clippy::missing_errors_doc)]
pub fn parse_from_fs(path: impl AsRef<Path>) -> Result<Config, ParseError> {
    let raw = fs::read_to_string(path)?;
    let cfg = toml::from_str(&raw)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::YtDlpConfig;

    #[test]
    fn command_parts_use_explicit_command_when_present() {
        let config = YtDlpConfig {
            command: vec!["python3".into(), "-m".into(), "yt_dlp".into()],
            executable_path: Some("./yt-dlp/executable".into()),
            plugin_dirs: vec![],
            cookies_path: "./cookies".into(),
            max_file_size: 1,
        };

        let (program, args) = config.command_parts();
        assert_eq!(program, "python3");
        assert_eq!(args, vec!["-m", "yt_dlp"]);
    }

    #[test]
    fn command_parts_fall_back_to_legacy_executable_path() {
        let config = YtDlpConfig {
            command: vec![],
            executable_path: Some("./yt-dlp/executable".into()),
            plugin_dirs: vec![],
            cookies_path: "./cookies".into(),
            max_file_size: 1,
        };

        let (program, args) = config.command_parts();
        assert_eq!(program, "./yt-dlp/executable");
        assert!(args.is_empty());
    }

    #[test]
    fn command_parts_default_to_python_module_launcher() {
        let config = YtDlpConfig {
            command: vec![],
            executable_path: None,
            plugin_dirs: vec![],
            cookies_path: "./cookies".into(),
            max_file_size: 1,
        };

        let (program, args) = config.command_parts();
        assert_eq!(program, "python3");
        assert_eq!(args, vec!["-m", "yt_dlp"]);
    }
}
