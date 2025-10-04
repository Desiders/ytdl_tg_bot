use std::{borrow::Cow, fmt::Display};
use url::Host;

use super::AspectKind;

pub fn get_url_by_aspect<'a>(
    service_host: Option<&Host<&str>>,
    id: impl Display,
    urls: &[&'a str],
    aspect_kind: AspectKind,
) -> Option<Cow<'a, str>> {
    let domain = match service_host {
        Some(host) => match host {
            Host::Domain(domain) => domain,
            _ => return None,
        },
        None => return None,
    };

    if domain.contains("youtube") || *domain == "youtu.be" {
        let accepted_fragments = match aspect_kind {
            AspectKind::Vertical => vec!["oardefault.jpg"],
            AspectKind::Sd => vec!["sddefault.jpg", "0.jpg", "hqdefault.jpg"],
            AspectKind::Hd => vec!["maxresdefault.jpg", "hq720.jpg", "maxres2.jpg"],
            AspectKind::Other => return Some(Cow::Owned(format!("https://i.ytimg.com/vi/{id}/frame0.jpg"))),
        };

        for url in urls {
            for fragment in &accepted_fragments {
                if url.contains(fragment) {
                    return Some(Cow::Borrowed(url));
                }
            }
        }
    }

    None
}
