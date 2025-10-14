use telers::{
    errors::EventErrorKind,
    event::telegram::HandlerResponse,
    methods::SetMessageReaction,
    middlewares::{inner::Middleware, Next},
    types::ReactionTypeEmoji,
    Request,
};
use tracing::{event, Level};

static REACTIONS: [&str; 2] = ["👌", "👍"];

#[derive(Clone)]
pub struct ReactionMiddleware;

impl Middleware for ReactionMiddleware {
    async fn call(&mut self, request: Request, next: Next) -> Result<HandlerResponse, EventErrorKind> {
        let Some(message) = request.update.message() else {
            return next(request).await;
        };
        let bot = request.bot.clone();
        let message_id = message.id();
        let chat_id = message.chat().id();

        let ((), resp) = tokio::join!(
            {
                let bot = bot.clone();
                async move {
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
                                event!(Level::ERROR, %err, reaction, "Set reaction err");
                            }
                        }
                    }
                }
            },
            next(request)
        );

        tokio::spawn(async move {
            match bot.send(SetMessageReaction::new(chat_id, message_id)).await {
                Ok(_) => {}
                Err(err) => {
                    event!(Level::ERROR, %err, "Unset reaction err");
                }
            }
        });

        resp
    }
}
