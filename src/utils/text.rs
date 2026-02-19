use telers::utils::text::html_text_link;
use url::Url;

fn invisible_link(webpage_url: Option<&Url>) -> Option<String> {
    webpage_url.map(|url| html_text_link("&#8203;&#8203;", url))
}

pub fn media_link(webpage_url: Option<&Url>) -> Option<String> {
    let mut text = String::new();
    if let Some(invisible_link) = invisible_link(webpage_url) {
        text.push_str(&invisible_link);
    }
    if let Some(url) = webpage_url {
        text.push_str(&html_text_link("Link", url));
    }
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}
