use std::{
    io::{Error, ErrorKind},
    time::Duration,
};

use redis::{aio::ConnectionManager, RedisResult};
use tokio::time::timeout;

const CACHE_TTL_SECS: u64 = 900;
const CACHE_TIMEOUT: Duration = Duration::from_secs(2);

pub struct MenuMessageCache {
    conn: ConnectionManager,
}

impl MenuMessageCache {
    pub fn new(conn: ConnectionManager) -> Self {
        Self { conn }
    }

    pub async fn get(&self, chat_id: i64, user_id: i64) -> RedisResult<Option<i64>> {
        timeout(CACHE_TIMEOUT, async {
            redis::cmd("GET")
                .arg(key(chat_id, user_id))
                .query_async(&mut self.conn.clone())
                .await
        })
        .await
        .map_err(|err| Error::new(ErrorKind::TimedOut, err))?
    }

    pub async fn set(&self, chat_id: i64, user_id: i64, message_id: i64) -> RedisResult<()> {
        timeout(CACHE_TIMEOUT, async {
            redis::cmd("SET")
                .arg(key(chat_id, user_id))
                .arg(message_id)
                .arg("EX")
                .arg(CACHE_TTL_SECS)
                .query_async(&mut self.conn.clone())
                .await
        })
        .await
        .map_err(|err| Error::new(ErrorKind::TimedOut, err))?
    }

    pub async fn remove(&self, chat_id: i64, user_id: i64) -> RedisResult<()> {
        timeout(CACHE_TIMEOUT, async {
            redis::cmd("DEL")
                .arg(key(chat_id, user_id))
                .query_async(&mut self.conn.clone())
                .await
        })
        .await
        .map_err(|err| Error::new(ErrorKind::TimedOut, err))?
    }
}

fn key(chat_id: i64, user_id: i64) -> String {
    format!("tg:menu:domain-input:{chat_id}:{user_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::queue::test_support::Fixture;
    use tokio::time::Instant;

    #[tokio::test]
    async fn menu_messages_are_scoped_by_chat_and_user_and_expire() {
        let fixture = Fixture::new().await;
        let mut conn = fixture.connection();
        let cache = MenuMessageCache::new(conn.clone());

        assert_eq!(cache.get(-1, 1).await.unwrap(), None);
        cache.set(-1, 1, 7).await.unwrap();
        assert_eq!(cache.get(-1, 1).await.unwrap(), Some(7));
        assert_eq!(cache.get(-2, 1).await.unwrap(), None);
        assert_eq!(cache.get(-1, 2).await.unwrap(), None);

        let ttl: i64 = redis::cmd("TTL").arg(key(-1, 1)).query_async(&mut conn).await.unwrap();
        assert!(ttl > 0 && ttl <= i64::try_from(CACHE_TTL_SECS).unwrap());

        cache.remove(-1, 1).await.unwrap();
        assert_eq!(cache.get(-1, 1).await.unwrap(), None);
    }

    #[tokio::test]
    async fn stalled_cache_requests_have_a_deadline() {
        let fixture = Fixture::new().await;
        let mut conn = fixture.connection();
        let cache = MenuMessageCache::new(conn.clone());
        let _: () = redis::cmd("CLIENT")
            .arg("PAUSE")
            .arg(10_000)
            .arg("ALL")
            .query_async(&mut conn)
            .await
            .unwrap();

        let started = Instant::now();
        assert!(cache.get(-1, 1).await.unwrap_err().is_timeout());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
