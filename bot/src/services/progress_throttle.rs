use std::time::Duration;

use redis::aio::ConnectionManager;

pub struct ProgressThrottle {
    conn: ConnectionManager,
}

impl ProgressThrottle {
    pub fn new(conn: ConnectionManager) -> Self {
        Self { conn }
    }

    pub async fn try_acquire(&self, key: &str, interval: Duration) -> redis::RedisResult<bool> {
        let acquired: Option<String> = redis::cmd("SET")
            .arg(key)
            .arg("1")
            .arg("NX")
            .arg("PX")
            .arg(u64::try_from(interval.as_millis()).unwrap())
            .query_async(&mut self.conn.clone())
            .await?;
        Ok(acquired.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::queue::test_support::Fixture;
    use tokio::task::JoinSet;
    use uuid::Uuid;

    #[tokio::test]
    async fn progress_from_concurrent_downloads_shares_one_chat_budget() {
        let fixture = Fixture::new().await;
        let conn = fixture.connection();
        let key = format!("test:{}:chat:1", Uuid::now_v7());
        let mut tasks = JoinSet::new();
        for _ in 0..5 {
            let conn = conn.clone();
            let key = key.clone();
            tasks.spawn(async move { ProgressThrottle::new(conn).try_acquire(&key, Duration::from_secs(5)).await.unwrap() });
        }
        let mut accepted = 0;
        while let Some(result) = tasks.join_next().await {
            accepted += usize::from(result.unwrap());
        }
        assert_eq!(accepted, 1);
        assert!(ProgressThrottle::new(conn)
            .try_acquire(&format!("{key}:other"), Duration::from_secs(5))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn expired_progress_entries_are_removed() {
        let fixture = Fixture::new().await;
        let mut conn = fixture.connection();
        let key = format!("test:{}:progress", Uuid::now_v7());
        let interval = Duration::from_millis(200);
        let throttle = ProgressThrottle::new(conn.clone());
        assert!(throttle.try_acquire(&key, interval).await.unwrap());
        assert!(!throttle.try_acquire(&key, interval).await.unwrap());
        let ttl: i64 = redis::cmd("PTTL").arg(&key).query_async(&mut conn).await.unwrap();
        assert!(ttl > 0 && ttl <= 200);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let exists: bool = redis::cmd("EXISTS").arg(&key).query_async(&mut conn).await.unwrap();
                if !exists {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(throttle.try_acquire(&key, interval).await.unwrap());
    }
}
