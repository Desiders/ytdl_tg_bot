#![allow(clippy::module_name_repetitions)]

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
    pub node_tokens: Vec<Box<str>>,
    pub cookie_manager_token: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct TlsConfig {
    #[serde(rename = "ca_cert_path")]
    pub ca_cert: Box<str>,
    #[serde(rename = "cert_path")]
    pub cert: Box<str>,
    #[serde(rename = "key_path")]
    pub key: Box<str>,
}

#[derive(Default, Deserialize, Clone, Debug)]
pub struct YtDlpConfig {
    #[serde(default)]
    pub command: Vec<Box<str>>,
    pub max_file_size: u64,
    #[serde(default = "default_extractor_args")]
    pub extractor_args: Box<str>,
}

impl YtDlpConfig {
    #[must_use]
    pub fn command_parts(&self) -> (&str, Vec<&str>) {
        if let Some((program, args)) = self.command.split_first() {
            return (program.as_ref(), args.iter().map(AsRef::as_ref).collect());
        }
        ("python3", vec!["-m", "yt_dlp"])
    }
}

fn default_extractor_args() -> Box<str> {
    "youtube:player_client=default,mweb;player_skip=configs,initial_data;use_ad_playback_context=true".into()
}

#[derive(Default, Deserialize, Clone, Debug)]
pub struct GalleryDlConfig {
    #[serde(default)]
    pub command: Vec<Box<str>>,
}

impl GalleryDlConfig {
    #[must_use]
    pub fn command_parts(&self) -> (&str, Vec<&str>) {
        if let Some((program, args)) = self.command.split_first() {
            return (program.as_ref(), args.iter().map(AsRef::as_ref).collect());
        }
        ("python3", vec!["-m", "gallery_dl"])
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct YtPotProviderConfig {
    pub url: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ReplaceRule {
    pub from: Box<str>,
    pub to: Box<str>,
}

#[derive(Default, Deserialize, Clone, Debug)]
pub struct ReplaceDomainsConfig {
    #[serde(default)]
    pub video: Vec<ReplaceRule>,
    #[serde(default)]
    pub audio: Vec<ReplaceRule>,
    #[serde(default)]
    pub photo: Vec<ReplaceRule>,
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
    pub gallery_dl: GalleryDlConfig,
    pub yt_pot_provider: YtPotProviderConfig,
    pub logging: LoggingConfig,
    #[serde(default)]
    pub replace_domains: ReplaceDomainsConfig,
}

#[derive(Error, Debug)]
pub enum TlsLoadError {
    #[error("Failed to read downloader CA certificate {path}: {source}")]
    Ca { path: Box<str>, source: io::Error },
    #[error("Failed to read node certificate {path}: {source}")]
    Cert { path: Box<str>, source: io::Error },
    #[error("Failed to read node key {path}: {source}")]
    Key { path: Box<str>, source: io::Error },
}

impl Config {
    pub fn load_server_tls_cfg(&self) -> Result<ServerTlsConfig, TlsLoadError> {
        let config = &self.tls;
        let ca_cert_pem = fs::read(&*config.ca_cert).map_err(|source| TlsLoadError::Ca {
            path: config.ca_cert.clone(),
            source,
        })?;
        let cert_pem = fs::read(&*config.cert).map_err(|source| TlsLoadError::Cert {
            path: config.cert.clone(),
            source,
        })?;
        let key_pem = fs::read(&*config.key).map_err(|source| TlsLoadError::Key {
            path: config.key.clone(),
            source,
        })?;

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

/// Loads downloader configuration from a TOML file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or the TOML cannot be parsed.
pub fn parse_from_fs(path: impl AsRef<Path>) -> Result<Config, ParseError> {
    let raw = fs::read_to_string(path)?;
    let cfg = toml::from_str(&raw)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::{GalleryDlConfig, YtDlpConfig};

    #[test]
    fn command_parts_use_explicit_command_when_present() {
        let config = YtDlpConfig {
            command: vec!["python3".into(), "-m".into(), "yt_dlp".into()],
            max_file_size: 1,
            extractor_args: super::default_extractor_args(),
        };

        let (program, args) = config.command_parts();
        assert_eq!(program, "python3");
        assert_eq!(args, vec!["-m", "yt_dlp"]);
    }

    #[test]
    fn command_parts_default_to_python_module_launcher() {
        let config = YtDlpConfig {
            command: vec![],
            max_file_size: 1,
            extractor_args: super::default_extractor_args(),
        };

        let (program, args) = config.command_parts();
        assert_eq!(program, "python3");
        assert_eq!(args, vec!["-m", "yt_dlp"]);
    }

    #[test]
    fn gallery_command_parts_use_explicit_command_when_present() {
        let config = GalleryDlConfig {
            command: vec!["python3".into(), "-m".into(), "gallery_dl".into()],
        };

        let (program, args) = config.command_parts();
        assert_eq!(program, "python3");
        assert_eq!(args, vec!["-m", "gallery_dl"]);
    }

    #[test]
    fn gallery_command_parts_default_to_python_module_launcher() {
        let config = GalleryDlConfig { command: vec![] };

        let (program, args) = config.command_parts();
        assert_eq!(program, "python3");
        assert_eq!(args, vec!["-m", "gallery_dl"]);
    }

    #[test]
    fn replace_domains_config_parses_per_media_type() {
        let raw = r"
            [[video]]
            from = 'a'
            to = 'b'

            [[audio]]
            from = 'c'
            to = 'd'
        ";

        let config: super::ReplaceDomainsConfig = toml::from_str(raw).unwrap();

        assert_eq!(config.video.len(), 1);
        assert_eq!(&*config.video[0].from, "a");
        assert_eq!(&*config.video[0].to, "b");
        assert_eq!(config.audio.len(), 1);
        assert!(config.photo.is_empty());
    }

    #[test]
    fn replace_domains_config_defaults_to_empty() {
        let config: super::ReplaceDomainsConfig = toml::from_str("").unwrap();

        assert!(config.video.is_empty());
        assert!(config.audio.is_empty());
        assert!(config.photo.is_empty());
    }
}
