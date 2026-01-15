use crate::config::DomainsWithReactionsConfig;

use froodi::async_impl::Container;
use telers::{
    errors::EventErrorKind,
    event::telegram::HandlerResponse,
    methods::SetMessageReaction,
    middlewares::{inner::Middleware, Next},
    types::ReactionTypeEmoji,
    Request,
};
use tracing::error;
use url::Url;

static REACTIONS: [&str; 2] = ["👌", "👍"];

#[derive(Clone)]
pub struct ReactionMiddleware;

impl Middleware for ReactionMiddleware {
    async fn call(&mut self, request: Request, next: Next) -> Result<HandlerResponse, EventErrorKind> {
        let Some(message) = request.update.message() else {
            return next(request).await;
        };
        let Some(domain) = request.extensions.get::<Url>().and_then(|url| url.domain()) else {
            return next(request).await;
        };
        let container = request.extensions.get::<Container>().unwrap();
        let domains_with_reactions = container.get::<DomainsWithReactionsConfig>().await.unwrap();

        if !domains_with_reactions
            .domains
            .contains(&domain.trim_start_matches("www.").to_owned())
        {
            return next(request).await;
        }

        let bot = request.bot.clone();
        let message_id = message.id();
        let chat_id = message.chat().id();

        for reaction in REACTIONS {
            match bot
                .send(
                    SetMessageReaction::new(chat_id, message_id)
                        .reaction(ReactionTypeEmoji::new(reaction))
                        .is_big(false),
                )
                .await
            {
                Ok(_) => break,
                Err(err) => {
                    error!(%err, reaction, "Set reaction err");
                }
            }
        }

        let resp = next(request).await;

        tokio::spawn(async move {
            match bot.send(SetMessageReaction::new(chat_id, message_id)).await {
                Ok(_) => {}
                Err(err) => {
                    error!(%err, "Unset reaction err");
                }
            }
        });

        resp
    }
}
