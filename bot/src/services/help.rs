use rust_i18n::t;
use telers::utils::text::{html_quote, html_text_link};

use crate::locale::Locale;

pub fn commands(locale: Locale) -> String {
    t!("help.commands", locale = locale.as_str()).into_owned()
}

pub fn inline(locale: Locale, username: &str) -> String {
    t!("help.inline", locale = locale.as_str(), username = username).into_owned()
}

pub fn arguments(locale: Locale, max_file_size_in_mb: u64, src_url: &str) -> String {
    let source_label = t!("start.source_code_label", locale = locale.as_str());
    let source_code = html_text_link(source_label.as_ref(), html_quote(src_url));

    t!(
        "help.arguments",
        locale = locale.as_str(),
        max_file_size_in_mb = max_file_size_in_mb,
        source_code = source_code,
    )
    .into_owned()
}

/// Groups have no menu, so `/start` there shows every help page at once.
pub fn full(locale: Locale, username: &str, max_file_size_in_mb: u64, src_url: &str) -> String {
    [
        commands(locale),
        inline(locale, username),
        arguments(locale, max_file_size_in_mb, src_url),
    ]
    .join("\n\n")
}
