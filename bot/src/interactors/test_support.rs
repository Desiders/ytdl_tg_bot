use std::sync::Arc;

use downloader_client::{DownloaderClusterConfig, DownloaderServiceTarget, DownloaderTlsConfig, NodeRouter};

use crate::config::Config;

pub fn config() -> Config {
    serde_json::from_value(serde_json::json!({
        "bot": {"token": "test", "src_url": "https://example.test"},
        "chat": {"receiver_chat_id": 1}, "logging": {"dirs": "info"},
        "database": {"host": "unused", "port": 5432, "user": "", "password": "", "database": ""},
        "redis": {"host": "unused", "port": 6379},
        "yt_dlp": {"max_file_size": 1_000_000}, "yt_toolkit": {"url": "https://example.test"},
        "download": {"node_token": "test", "tls": {"ca_cert_path": "", "cert_path": "", "key_path": ""}},
        "telegram_bot_api": {"url": "https://example.test"}
    }))
    .unwrap()
}

pub fn router(port: u16) -> Arc<NodeRouter> {
    Arc::new(NodeRouter::new(
        &DownloaderClusterConfig {
            node_token: "test".into(),
            tls: DownloaderTlsConfig {
                ca_cert: concat!(env!("CARGO_MANIFEST_DIR"), "/src/interactors/test_data/cert.pem").into(),
                cert: concat!(env!("CARGO_MANIFEST_DIR"), "/src/interactors/test_data/cert.pem").into(),
                key: concat!(env!("CARGO_MANIFEST_DIR"), "/src/interactors/test_data/key.pem").into(),
            },
        },
        1_000_000,
        Arc::new(DownloaderServiceTarget {
            host: "127.0.0.1".into(),
            port,
        }),
    ))
}
