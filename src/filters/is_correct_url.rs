use std::future::Future;
use telers::{types::Update, Bot, Context};
use tracing::{event, Level};
use url::Url;

pub fn is_correct_url(_bot: &Bot, update: &Update, _context: &Context) -> impl Future<Output = bool> {
    let result = if let Some(text) = update.text() {
        match Url::parse(text.trim()) {
            Ok(_) => true,
            Err(err) => {
                event!(Level::TRACE, %err, "Error while parsing url");

                false
            }
        }
    } else {
        false
    };

    async move { result }
}
