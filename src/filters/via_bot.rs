use std::future::Future;

use telers::{types::Update, Bot, Context};

#[allow(clippy::module_name_repetitions)]
pub fn is_via_bot(_bot: &Bot, update: &Update, _context: &Context) -> impl Future<Output = bool> {
    let result = update.message().map(|message| message.via_bot()).flatten().is_some();

    async move { result }
}
