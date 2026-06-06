use async_trait::async_trait;
use std::convert::Infallible;

use crate::{
    entities::{Chat, ChatStats},
    errors::ErrorKind,
};

#[async_trait]
pub trait ChatReader: Send + Sync {
    async fn get_stats(&self) -> Result<ChatStats, ErrorKind<Infallible>>;
}

#[async_trait]
pub trait ChatRepo: Send + Sync {
    async fn insert_or_update(&self, chat: Chat) -> Result<Chat, ErrorKind<Infallible>>;
}
