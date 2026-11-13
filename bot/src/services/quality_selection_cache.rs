use std::{
    io::{Error, ErrorKind},
    time::Duration,
};

use redis::{aio::ConnectionManager, RedisResult};
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

use crate::entities::DownloadJob;
use uuid::Uuid;

const CACHE_TTL_SECS: u64 = 900;
const CACHE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualitySelection {
    pub job: DownloadJob,
    pub catalog_id: Uuid,
}

pub struct QualitySelectionCache {
    conn: ConnectionManager,
}

impl QualitySelectionCache {
    pub fn new(conn: ConnectionManager) -> Self {
        Self { conn }
    }

    pub async fn set(&self, user_id: i64, session: &str, selection: &QualitySelection) -> RedisResult<()> {
        let payload = serde_json::to_string(selection).map_err(|err| Error::new(ErrorKind::InvalidData, err))?;
        timeout(CACHE_TIMEOUT, async {
            redis::cmd("SET")
                .arg(key(user_id, session))
                .arg(payload)
                .arg("EX")
                .arg(CACHE_TTL_SECS)
                .query_async(&mut self.conn.clone())
                .await
        })
        .await
        .map_err(|err| Error::new(ErrorKind::TimedOut, err))?
    }

    pub async fn get(&self, user_id: i64, session: &str) -> RedisResult<Option<QualitySelection>> {
        self.read(user_id, session, "GET").await
    }

    pub async fn take(&self, user_id: i64, session: &str) -> RedisResult<Option<QualitySelection>> {
        self.read(user_id, session, "GETDEL").await
    }

    async fn read(&self, user_id: i64, session: &str, command: &str) -> RedisResult<Option<QualitySelection>> {
        // GETDEL consumes the choice before queueing: duplicate clicks cannot enqueue another job,
        // including when the queue write has an uncertain outcome.
        let payload: Option<String> = timeout(CACHE_TIMEOUT, async {
            redis::cmd(command)
                .arg(key(user_id, session))
                .query_async(&mut self.conn.clone())
                .await
        })
        .await
        .map_err(|err| Error::new(ErrorKind::TimedOut, err))??;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(|err| Error::new(ErrorKind::InvalidData, err).into()))
            .transpose()
    }
}

fn key(user_id: i64, session: &str) -> String {
    format!("tg:quality:{user_id}:{session}")
}
