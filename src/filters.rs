mod text_contains_url;
mod text_empty;
mod via_bot;

pub use text_contains_url::{text_contains_url, text_contains_url_with_reply, url_is_blacklisted};
pub use text_empty::text_empty;
pub use via_bot::is_via_bot;
