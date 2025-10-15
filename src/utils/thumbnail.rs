use std::fmt::Display;
use tracing::instrument;
use url::Host;

use super::AspectKind;

#[instrument(skip_all, fields(%id, ?aspect_kind, ?host = service_host))]
pub fn get_urls_by_aspect(service_host: Option<&Host<&str>>, id: impl Display, aspect_kind: AspectKind) -> Vec<String> {
    let Some(Host::Domain(domain)) = service_host else {
        return vec![];
    };

    if domain.contains("youtube") || *domain == "youtu.be" {
        let fragments = match aspect_kind {
            AspectKind::Vertical => vec!["oardefault"],
            AspectKind::Sd => vec!["sddefault", "0", "hqdefault"],
            AspectKind::Hd => vec!["maxresdefault", "hq720", "maxres2"],
            AspectKind::Other => vec![],
        };

        return fragments
            .into_iter()
            .chain(Some("frame0"))
            .map(|fragment| format!("https://i.ytimg.com/vi/{id}/{fragment}.jpg"))
            .collect();
    }

    vec![]
}
