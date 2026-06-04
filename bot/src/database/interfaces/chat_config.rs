use async_trait::async_trait;
use std::convert::Infallible;

use crate::{
    entities::{ChatConfig, ChatConfigExcludeDomain, ChatConfigExcludeDomains, ChatConfigUpdate},
    errors::ErrorKind,
};

#[async_trait]
pub trait ChatConfigReader: Send + Sync {
    async fn get(&self, tg_id: i64) -> Result<Option<ChatConfig>, ErrorKind<Infallible>>;
    async fn get_exclude_domains(&self, tg_id: i64) -> Result<ChatConfigExcludeDomains, ErrorKind<Infallible>>;
}

#[async_trait]
pub trait ChatConfigRepo: Send + Sync {
    async fn insert_or_update(&self, config: ChatConfig) -> Result<ChatConfig, ErrorKind<Infallible>>;
    async fn update(&self, dto: ChatConfigUpdate) -> Result<ChatConfig, ErrorKind<Infallible>>;
    async fn insert_exclude_domain_or_update(&self, dto: ChatConfigExcludeDomain)
        -> Result<ChatConfigExcludeDomain, ErrorKind<Infallible>>;
    async fn delete_exclude_domain(&self, dto: ChatConfigExcludeDomain) -> Result<bool, ErrorKind<Infallible>>;
}
