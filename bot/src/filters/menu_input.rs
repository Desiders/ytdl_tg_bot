use std::{convert::Infallible, future::Future};
use telers::{filters::CommandObject, FilterResult, Request};

pub fn is_command(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let result = request.update.text().is_some_and(text_is_command);
    async move { Ok(result) }
}

// `Command` trims leading whitespace, so a bare `starts_with('/')` let ` /vd <url>` fill the prompt instead of running.
//  https://github.com/Desiders/telers/blob/a14a34520eaeffccd1ebdbb4fffe8e44ead81bb9/telers/src/filters/command.rs#L744-L795
fn text_is_command(text: &str) -> bool {
    CommandObject::extract(text).is_some_and(|command| command.prefix == '/')
}

// `/lang` without a language opens the picker; with one it switches directly.
pub fn command_without_args(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let result = request
        .update
        .text()
        .and_then(CommandObject::extract)
        .is_some_and(|command| command.args.is_empty());
    async move { Ok(result) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_recognized_with_leading_whitespace() {
        for text in [
            "/lang en",
            " /vd https://example.com/video",
            "\t/rm_ed example.com",
            "\n/add_ed example.com",
        ] {
            assert!(text_is_command(text), "{text:?}");
        }
        for text in ["example.com", "https://example.com/video", "/"] {
            assert!(!text_is_command(text), "{text:?}");
        }
    }
}
