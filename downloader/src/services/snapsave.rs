use snapsave_parser::SnapSave;
use tracing::warn;
use url::Url;

use crate::{config::SnapsaveConfig, services::domain_replacer::MediaKind};

pub struct ResolvedMedia {
    pub url: Url,
    pub thumbnail: Option<Url>,
}

pub enum SnapsaveOutcome {
    Resolved(Vec<ResolvedMedia>),
    WrongKind,
    Unavailable,
}

#[derive(Clone)]
pub struct SnapsaveResolver {
    enabled: bool,
    proxy: Option<String>,
}

impl SnapsaveResolver {
    #[must_use]
    pub fn new(cfg: &SnapsaveConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            proxy: cfg.proxy.as_deref().map(ToOwned::to_owned),
        }
    }

    #[must_use]
    pub fn is_supported(&self, url: &Url) -> bool {
        self.enabled && is_instagram_or_facebook(url)
    }

    pub async fn resolve(&self, url: &Url, kind: MediaKind) -> SnapsaveOutcome {
        if !self.enabled {
            return SnapsaveOutcome::Unavailable;
        }

        let target = url.to_string();
        let proxy = self.proxy.clone();
        let wanted_type = if matches!(kind, MediaKind::Photo) { "image" } else { "video" };

        let resolved = tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
            runtime.block_on(async move {
                let snap = match proxy.as_deref() {
                    Some(proxy) => SnapSave::with_proxy(proxy),
                    None => SnapSave::new(),
                }
                .map_err(|err| warn!(url = %target, %err, "Failed to build snapsave parser"))
                .ok()?;
                let data = match snap.download(&target, None).await {
                    Ok(data) => data,
                    Err(err) => {
                        warn!(url = %target, %err, "Snapsave resolve failed");
                        return None;
                    }
                };
                let preview = data.preview.as_deref().and_then(|preview| Url::parse(preview).ok());
                let items: Vec<ResolvedMedia> = data
                    .media
                    .into_iter()
                    // Render-gated entries (Facebook hi-res variants) need a server-side render, not a
                    // direct URL, so skip them.
                    .filter(|media| media.should_render != Some(true))
                    .filter(|media| media.r#type.as_deref() == Some(wanted_type))
                    .filter_map(|media| {
                        let url = Url::parse(media.url.as_deref()?).ok()?;
                        let thumbnail = media
                            .thumbnail
                            .as_deref()
                            .and_then(|thumbnail| Url::parse(thumbnail).ok())
                            .or_else(|| preview.clone());
                        Some(ResolvedMedia { url, thumbnail })
                    })
                    .collect();
                Some(items)
            })
        })
        .await;

        match resolved {
            Ok(Some(items)) if !items.is_empty() => SnapsaveOutcome::Resolved(items),
            Ok(Some(_)) => SnapsaveOutcome::WrongKind,
            Ok(None) | Err(_) => SnapsaveOutcome::Unavailable,
        }
    }
}

fn is_instagram_or_facebook(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.strip_prefix("www.").unwrap_or(host);
    host == "instagram.com"
        || host.ends_with(".instagram.com")
        || host == "instagr.am"
        || host == "facebook.com"
        || host.ends_with(".facebook.com")
        || host == "fb.watch"
}
