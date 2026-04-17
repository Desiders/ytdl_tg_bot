mod config;
mod cookies;
mod service;

use std::time::Duration;

use downloader_client::{AssignmentNodeClient, DownloaderServiceTarget};
use service::CookieAssignmentService;
use tracing::info;
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let config_path = config::get_path();
    let config = config::parse_from_fs(&*config_path).unwrap();
    let service_target = DownloaderServiceTarget::from_env();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::builder().parse_lossy(config.logging.dirs.as_ref()))
        .init();

    info!(
        config_path = %config_path,
        log_filter = %config.logging.dirs,
        downloader_service_dns = %service_target.authority(),
        sync_interval = config.sync.interval,
        "Loaded cookie assignment config"
    );

    assert!(config.sync.interval > 0, "`sync.interval` must be greater than zero");

    let client = AssignmentNodeClient::load(
        &config.download.tls.clone().into(),
        service_target.host.as_ref(),
        config.download.cookie_manager_token.clone(),
    );
    let mut service = CookieAssignmentService::new(client, service_target);
    let mut interval = tokio::time::interval(Duration::from_secs(config.sync.interval));

    info!("Starting cookie assignment loop");
    loop {
        interval.tick().await;
        service.sync_cycle().await;
    }
}
