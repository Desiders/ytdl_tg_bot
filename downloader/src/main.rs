mod config;
mod entities;
mod grpc;
mod services;
mod utils;

use std::sync::{atomic::AtomicU32, Arc};
use tokio::sync::Semaphore;
use tonic::transport::Server;
use tracing::info;
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};
use ytdl_tg_bot_proto::downloader::{downloader_server::DownloaderServer, node_capabilities_server::NodeCapabilitiesServer};

use crate::{
    grpc::{auth::AuthInterceptor, capabilities::CapabilitiesService, downloader::DownloaderService},
    services::get_cookies_from_directory,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let config_path = config::get_path();
    let config = config::parse_from_fs(&*config_path).unwrap();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::builder().parse_lossy(config.logging.dirs.as_ref()))
        .init();

    info!(
        config_path = %config_path,
        address = %config.server.address,
        max_concurrent = config.server.max_concurrent,
        token_count = config.auth.tokens.len(),
        log_filter = %config.logging.dirs,
        "Loaded downloader config"
    );

    let cookies = Arc::new(get_cookies_from_directory(&*config.yt_dlp.cookies_path).unwrap_or_default());
    info!(cookie_host_count = cookies.get_hosts().len(), hosts = ?cookies.get_hosts(), "Cookies loaded");

    let active_downloads = Arc::new(AtomicU32::new(0));
    let semaphore = Arc::new(Semaphore::new(config.server.max_concurrent as usize));

    let capabilities_service = CapabilitiesService {
        cookies: cookies.clone(),
        active_downloads: active_downloads.clone(),
        max_concurrent: config.server.max_concurrent,
    };
    let downloader_service = DownloaderService {
        yt_dlp_cfg: Arc::new(config.yt_dlp),
        yt_pot_provider_cfg: Arc::new(config.yt_pot_provider),
        cookies: cookies.clone(),
        active_downloads: active_downloads.clone(),
        semaphore,
    };

    let auth = AuthInterceptor::new(config.auth.tokens);
    let addr = config.server.address.parse().unwrap();
    info!(%addr, "Starting download node");

    Server::builder()
        .add_service(DownloaderServer::with_interceptor(downloader_service, auth.clone()))
        .add_service(NodeCapabilitiesServer::with_interceptor(capabilities_service, auth))
        .serve_with_shutdown(addr, shutdown_signal())
        .await
        .unwrap();

    info!("Download node stopped");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("Failed to install Ctrl+C shutdown handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM shutdown handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => info!("Received Ctrl+C, shutting down download node"),
        () = terminate => info!("Received SIGTERM, shutting down download node"),
    }
}
