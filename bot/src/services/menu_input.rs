use std::time::Duration;

use redis::aio::ConnectionManager;

// A forgotten prompt expires instead of reading a later link as a domain.
const DOMAIN_INPUT_TTL: Duration = Duration::from_secs(300);

pub struct MenuInputState {
    conn: ConnectionManager,
}

impl MenuInputState {
    pub fn new(conn: ConnectionManager) -> Self {
        Self { conn }
    }

    pub async fn await_domain(&self, chat_id: i64) -> redis::RedisResult<()> {
        redis::cmd("SET")
            .arg(Self::key(chat_id))
            .arg("1")
            .arg("EX")
            .arg(DOMAIN_INPUT_TTL.as_secs())
            .query_async(&mut self.conn.clone())
            .await
    }

    // `DEL` answers 1 to one caller only, so concurrent messages cannot all take the same prompt.
    pub async fn claim_domain_input(&self, chat_id: i64) -> redis::RedisResult<bool> {
        let removed: i64 = redis::cmd("DEL")
            .arg(Self::key(chat_id))
            .query_async(&mut self.conn.clone())
            .await?;
        Ok(removed == 1)
    }

    pub async fn clear(&self, chat_id: i64) -> redis::RedisResult<()> {
        redis::cmd("DEL").arg(Self::key(chat_id)).query_async(&mut self.conn.clone()).await
    }

    fn key(chat_id: i64) -> String {
        format!("menu:{chat_id}:domain_input")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::queue::test_support::Fixture;

    #[tokio::test]
    async fn domain_input_is_awaited_with_a_ttl_until_claimed() {
        let fixture = Fixture::new().await;
        let mut conn = fixture.connection();
        let state = MenuInputState::new(conn.clone());
        let chat_id = 1;

        assert!(!state.claim_domain_input(chat_id).await.unwrap());

        state.await_domain(chat_id).await.unwrap();

        let ttl: i64 = redis::cmd("TTL")
            .arg(MenuInputState::key(chat_id))
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!(ttl > 0 && ttl <= 300);

        assert!(state.claim_domain_input(chat_id).await.unwrap());
        assert!(!state.claim_domain_input(chat_id).await.unwrap());
    }

    #[tokio::test]
    async fn one_prompt_is_claimed_by_one_of_concurrent_messages() {
        let fixture = Fixture::new().await;
        let state = MenuInputState::new(fixture.connection());
        let chat_id = 1;
        state.await_domain(chat_id).await.unwrap();

        let claims = futures_util::future::join_all((0..8).map(|_| state.claim_domain_input(chat_id))).await;

        assert_eq!(claims.into_iter().filter(|claim| *claim.as_ref().unwrap()).count(), 1);
    }
}
