//! Durable download queue backed by a Redis Stream + consumer group.
//!
//! `enqueue` appends a [`DownloadJob`] (`XADD`); workers pull with `XREADGROUP` (each a distinct
//! consumer), `ack` on success (`XACK` + `XDEL`), and crashed-worker jobs are recovered with
//! `XAUTOCLAIM` ([`reclaim_stale`]). A per-`job_id` "done" marker gives best-effort dedup so a job
//! replayed after a crash-before-ack is not delivered twice.

use std::sync::Arc;

use redis::{
    aio::ConnectionManager,
    streams::{StreamAutoClaimReply, StreamReadReply},
    AsyncCommands as _, RedisError, Value,
};
use tracing::error;
use uuid::Uuid;

use crate::{config::QueueConfig, entities::DownloadJob};

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error(transparent)]
    Redis(#[from] RedisError),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("Stream entry {entry_id} is missing the `data` field")]
    MissingPayload { entry_id: String },
}

/// A job read off the stream together with the stream entry id needed to `ack` it.
pub struct QueuedJob {
    pub entry_id: String,
    pub job: DownloadJob,
}

pub struct RedisJobQueue {
    conn: ConnectionManager,
    cfg: Arc<QueueConfig>,
}

impl RedisJobQueue {
    #[must_use]
    pub fn new(conn: ConnectionManager, cfg: Arc<QueueConfig>) -> Self {
        Self { conn, cfg }
    }

    /// Creates the consumer group (and the stream) if it does not exist yet. Idempotent.
    pub async fn ensure_group(&self) -> Result<(), QueueError> {
        let mut conn = self.conn.clone();
        let res: Result<(), RedisError> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(&*self.cfg.stream_key)
            .arg(&*self.cfg.group)
            .arg("$")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;
        match res {
            Ok(()) => Ok(()),
            // group already exists — fine
            Err(err) if err.code() == Some("BUSYGROUP") => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    pub async fn enqueue(&self, job: &DownloadJob) -> Result<(), QueueError> {
        self.xadd(&self.cfg.stream_key, job).await
    }

    async fn xadd(&self, key: &str, job: &DownloadJob) -> Result<(), QueueError> {
        let payload = serde_json::to_string(job)?;
        let mut conn = self.conn.clone();
        let _: String = redis::cmd("XADD")
            .arg(key)
            .arg("*")
            .arg("data")
            .arg(payload)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    /// Blocks up to `block_ms` for the next undelivered job for this consumer.
    pub async fn read_next(&self, consumer: &str) -> Result<Option<QueuedJob>, QueueError> {
        let mut conn = self.conn.clone();
        let reply: StreamReadReply = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(&*self.cfg.group)
            .arg(consumer)
            .arg("COUNT")
            .arg(1)
            .arg("BLOCK")
            .arg(self.cfg.block_ms)
            .arg("STREAMS")
            .arg(&*self.cfg.stream_key)
            .arg(">")
            .query_async(&mut conn)
            .await?;

        let Some(entry) = reply.keys.into_iter().next().and_then(|key| key.ids.into_iter().next()) else {
            return Ok(None);
        };
        Ok(Some(parse_entry(entry.id, &entry.map)?))
    }

    /// Reclaims jobs whose owning consumer has been idle longer than `claim_min_idle_ms`
    /// (i.e. a crashed worker's in-flight jobs). Returns the batch claimed for this consumer.
    pub async fn reclaim_stale(&self, consumer: &str) -> Result<Vec<QueuedJob>, QueueError> {
        let mut conn = self.conn.clone();
        let reply: StreamAutoClaimReply = redis::cmd("XAUTOCLAIM")
            .arg(&*self.cfg.stream_key)
            .arg(&*self.cfg.group)
            .arg(consumer)
            .arg(self.cfg.claim_min_idle_ms)
            .arg("0")
            .arg("COUNT")
            .arg(64)
            .query_async(&mut conn)
            .await?;

        let mut jobs = Vec::with_capacity(reply.claimed.len());
        for entry in reply.claimed {
            match parse_entry(entry.id.clone(), &entry.map) {
                Ok(job) => jobs.push(job),
                // a malformed entry can never succeed — drop it so it doesn't loop forever
                Err(err) => {
                    error!(%err, entry_id = %entry.id, "Dropping unparseable reclaimed job");
                    self.ack(&entry.id).await?;
                }
            }
        }
        Ok(jobs)
    }

    /// Acknowledges and removes a processed entry from the stream.
    pub async fn ack(&self, entry_id: &str) -> Result<(), QueueError> {
        let mut conn = self.conn.clone();
        let _: i64 = conn.xack(&*self.cfg.stream_key, &*self.cfg.group, &[entry_id]).await?;
        let _: i64 = conn.xdel(&*self.cfg.stream_key, &[entry_id]).await?;
        Ok(())
    }

    /// Moves a job to the dead-letter stream and acks the original.
    pub async fn dead_letter(&self, job: &DownloadJob, entry_id: &str) -> Result<(), QueueError> {
        self.xadd(&self.cfg.dead_letter_key, job).await?;
        self.ack(entry_id).await
    }

    /// Re-enqueues a job (e.g. after a transient failure) with an incremented attempt count, and
    /// acks the original entry so it leaves this consumer's pending list.
    pub async fn requeue(&self, job: &DownloadJob, entry_id: &str) -> Result<(), QueueError> {
        self.enqueue(job).await?;
        self.ack(entry_id).await
    }

    /// True if this `job_id` was already completed (used to suppress duplicate sends on replay).
    pub async fn is_done(&self, job_id: Uuid) -> Result<bool, QueueError> {
        let mut conn = self.conn.clone();
        let exists: bool = conn.exists(done_key(job_id)).await?;
        Ok(exists)
    }

    /// Marks a `job_id` as completed, with a TTL so the marker set does not grow unbounded.
    pub async fn mark_done(&self, job_id: Uuid) -> Result<(), QueueError> {
        let mut conn = self.conn.clone();
        let _: () = conn.set_ex(done_key(job_id), 1, self.cfg.dedup_ttl_secs).await?;
        Ok(())
    }

    #[must_use]
    pub fn cfg(&self) -> &QueueConfig {
        &self.cfg
    }
}

fn done_key(job_id: Uuid) -> String {
    format!("ytdl:job:done:{job_id}")
}

fn parse_entry(entry_id: String, map: &std::collections::HashMap<String, Value>) -> Result<QueuedJob, QueueError> {
    let payload: String = match map.get("data") {
        Some(value) => redis::from_redis_value(value)?,
        None => return Err(QueueError::MissingPayload { entry_id }),
    };
    let job: DownloadJob = serde_json::from_str(&payload)?;
    Ok(QueuedJob { entry_id, job })
}
