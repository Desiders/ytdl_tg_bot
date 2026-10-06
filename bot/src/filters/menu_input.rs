use crate::services::menu_input::MenuInputState;

use froodi::async_impl::Container;
use std::{convert::Infallible, future::Future};
use telers::{filters::CommandObject, types::Chat, FilterResult, Request};
use tracing::error;

// A command typed while the prompt is open still runs as a command. Filters stop at the first `false`, so register this
//  one last: it consumes the prompt, and invalid input re-arms it by reopening the add screen.
//  https://github.com/Desiders/telers/blob/a14a34520eaeffccd1ebdbb4fffe8e44ead81bb9/telers/src/event/telegram/handler.rs#L101-L107
pub fn claims_domain_input(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let is_command = request.update.text().is_some_and(is_command);
    let container_option = request.extensions.get::<Container>().cloned();
    let chat_id = request.update.chat().map(Chat::id);
    async move {
        let (false, Some(container), Some(chat_id)) = (is_command, container_option, chat_id) else {
            return Ok(false);
        };
        let state = container.get::<MenuInputState>().await.unwrap();
        match state.claim_domain_input(chat_id).await {
            Ok(claimed) => Ok(claimed),
            Err(err) => {
                error!(%err, "Menu input state error");
                Ok(false)
            }
        }
    }
}

// `Command` trims leading whitespace, so a bare `starts_with('/')` let ` /vd <url>` fill the prompt instead of running.
//  https://github.com/Desiders/telers/blob/a14a34520eaeffccd1ebdbb4fffe8e44ead81bb9/telers/src/filters/command.rs#L744-L795
fn is_command(text: &str) -> bool {
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

// `CreateChatMiddleware` loads a callback's config by the pressing user, which matches the chat only in a private one.
pub fn is_private_callback(request: &mut Request) -> impl Future<Output = FilterResult<Infallible>> {
    let result = request
        .update
        .callback_query()
        .and_then(|query| query.message.as_deref())
        .is_some_and(|message| matches!(message.chat(), Chat::Private(_)));
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
            assert!(is_command(text), "{text:?}");
        }
        for text in ["example.com", "https://example.com/video", "/"] {
            assert!(!is_command(text), "{text:?}");
        }
    }
}
