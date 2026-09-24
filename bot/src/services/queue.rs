//! Durable download queue backed by a Redis Stream + consumer group.
//!
//! Redis owns delivery and crash recovery. Normal retries atomically replace their
//! stream entry; only uncertain work remains pending for `XAUTOCLAIM` recovery.

use std::sync::Arc;

use redis::{
    aio::ConnectionManager,
    streams::{StreamAutoClaimReply, StreamPendingReply, StreamReadReply},
    AsyncCommands as _, RedisError,
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

/// A job read off the stream together with its entry id and delivery origin.
pub struct QueuedJob {
    pub entry_id: String,
    pub recovered: bool,
    pub job: DownloadJob,
}

/// A point-in-time snapshot of the queue, for `/stats`.
#[derive(Debug, Default, Clone, Copy)]
pub struct QueueStats {
    pub waiting: u64,
    /// Delivered but not acknowledged. This can include quarantined work whose
    /// previous remote execution outcome is intentionally unknown.
    pub pending: u64,
    pub dead_letter: u64,
}

pub struct RedisJobQueue {
    conn: ConnectionManager,
    cfg: Arc<QueueConfig>,
}

const ACK_DELETE: &str = r"
local pending = redis.call('XPENDING', KEYS[1], ARGV[1], ARGV[2], ARGV[2], 1, ARGV[3])
if #pending == 0 then return 0 end
local acked = redis.call('XACK', KEYS[1], ARGV[1], ARGV[2])
if acked == 1 then redis.call('XDEL', KEYS[1], ARGV[2]) end
return acked
";

const RETRY: &str = r"
local pending = redis.call('XPENDING', KEYS[1], ARGV[1], ARGV[2], ARGV[2], 1, ARGV[3])
if #pending == 0 then return 0 end
local new_id = redis.call('XADD', KEYS[1], '*', 'data', ARGV[4])
local acked = redis.call('XACK', KEYS[1], ARGV[1], ARGV[2])
if acked == 1 then
  redis.call('XDEL', KEYS[1], ARGV[2])
  return 1
end
redis.call('XDEL', KEYS[1], new_id)
return 0
";

impl RedisJobQueue {
    #[must_use]
    pub fn new(conn: ConnectionManager, cfg: Arc<QueueConfig>) -> Self {
        Self { conn, cfg }
    }

    /// Creates the group at the first stream item so pre-start enqueues are consumed.
    pub async fn ensure_group(&self) -> Result<(), QueueError> {
        let mut conn = self.conn.clone();
        let res: Result<(), RedisError> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(&*self.cfg.stream_key)
            .arg(&*self.cfg.group)
            .arg("0")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;
        match res {
            Ok(()) => Ok(()),
            Err(err) if err.code() == Some("BUSYGROUP") => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    pub async fn enqueue(&self, job: &DownloadJob) -> Result<(), QueueError> {
        self.xadd(&self.cfg.stream_key, job).await
    }

    async fn xadd(&self, key: &str, job: &DownloadJob) -> Result<(), QueueError> {
        let mut conn = self.conn.clone();
        let _: String = redis::cmd("XADD")
            .arg(key)
            .arg("*")
            .arg("data")
            .arg(serde_json::to_string(job)?)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    /// Blocks up to `block_ms` for the next undelivered job for this consumer.
    pub async fn read_next(&self, conn: &mut ConnectionManager, consumer: &str) -> Result<Option<QueuedJob>, QueueError> {
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
            .query_async(conn)
            .await?;
        let Some(entry) = reply.keys.into_iter().next().and_then(|key| key.ids.into_iter().next()) else {
            return Ok(None);
        };
        let entry_id = entry.id.clone();
        match parse_entry(entry, false) {
            Ok(job) => Ok(Some(job)),
            Err(err) => {
                error!(%err, entry_id, "Dropping unparseable queued job");
                let _ = self.ack(&entry_id, consumer).await?;
                Ok(None)
            }
        }
    }

    /// Reclaims entries that have been pending longer than `claim_min_idle_ms`.
    pub async fn reclaim_stale(&self, consumer: &str, cursor: &mut String) -> Result<Vec<QueuedJob>, QueueError> {
        let mut conn = self.conn.clone();
        let reply: StreamAutoClaimReply = redis::cmd("XAUTOCLAIM")
            .arg(&*self.cfg.stream_key)
            .arg(&*self.cfg.group)
            .arg(consumer)
            .arg(self.cfg.claim_min_idle_ms)
            .arg(&*cursor)
            .arg("COUNT")
            .arg(1)
            .query_async(&mut conn)
            .await?;
        cursor.clone_from(&reply.next_stream_id);
        let mut jobs = Vec::with_capacity(reply.claimed.len());
        for entry in reply.claimed {
            let id = entry.id.clone();
            match parse_entry(entry, true) {
                Ok(job) => jobs.push(job),
                Err(err) => {
                    error!(%err, entry_id = %id, "Dropping unparseable reclaimed job");
                    let _ = self.ack(&id, consumer).await?;
                }
            }
        }
        Ok(jobs)
    }

    pub async fn ack(&self, entry_id: &str, consumer: &str) -> Result<bool, QueueError> {
        let mut conn = self.conn.clone();
        let acked: i64 = redis::cmd("EVAL")
            .arg(ACK_DELETE)
            .arg(1)
            .arg(&*self.cfg.stream_key)
            .arg(&*self.cfg.group)
            .arg(entry_id)
            .arg(consumer)
            .query_async(&mut conn)
            .await?;
        Ok(acked == 1)
    }

    /// Atomically replaces a terminally failed entry with its next application attempt.
    /// Returns false when an earlier call already transitioned this entry or it
    /// was reclaimed by another consumer.
    pub async fn retry(&self, job: &DownloadJob, entry_id: &str, consumer: &str) -> Result<bool, QueueError> {
        let mut conn = self.conn.clone();
        let transitioned: i64 = redis::cmd("EVAL")
            .arg(RETRY)
            .arg(1)
            .arg(&*self.cfg.stream_key)
            .arg(&*self.cfg.group)
            .arg(entry_id)
            .arg(consumer)
            .arg(serde_json::to_string(job)?)
            .query_async(&mut conn)
            .await?;
        Ok(transitioned == 1)
    }

    async fn is_owned(&self, entry_id: &str, consumer: &str) -> Result<bool, QueueError> {
        let mut conn = self.conn.clone();
        let pending: Vec<(String, String, u64, u64)> = redis::cmd("XPENDING")
            .arg(&*self.cfg.stream_key)
            .arg(&*self.cfg.group)
            .arg(entry_id)
            .arg(entry_id)
            .arg(1)
            .query_async(&mut conn)
            .await?;
        Ok(pending.first().is_some_and(|entry| entry.1 == consumer))
    }

    /// Publishes before acknowledgement so a publication failure leaves recoverable work.
    /// The queue and dead-letter streams can be in different Valkey Cluster slots, so
    /// this transfer cannot be atomic without changing the configured key layout.
    pub async fn dead_letter(&self, job: &DownloadJob, entry_id: &str, consumer: &str) -> Result<bool, QueueError> {
        if !self.is_owned(entry_id, consumer).await? {
            return Ok(false);
        }
        self.xadd(&self.cfg.dead_letter_key, job).await?;
        self.ack(entry_id, consumer).await
    }

    /// True if this `job_id` was already completed (best-effort replay deduplication).
    pub async fn is_done(&self, job_id: Uuid) -> Result<bool, QueueError> {
        Ok(self.conn.clone().exists(done_key(job_id)).await?)
    }

    /// Marks a job as completed before acknowledgement so crash recovery does not resend it.
    pub async fn mark_done(&self, job_id: Uuid) -> Result<(), QueueError> {
        let _: () = self.conn.clone().set_ex(done_key(job_id), 1, self.cfg.dedup_ttl_secs).await?;
        Ok(())
    }

    pub async fn stats(&self) -> Result<QueueStats, QueueError> {
        let mut conn = self.conn.clone();
        let total: u64 = conn.xlen(&*self.cfg.stream_key).await?;
        let pending: StreamPendingReply = redis::cmd("XPENDING")
            .arg(&*self.cfg.stream_key)
            .arg(&*self.cfg.group)
            .query_async(&mut conn)
            .await?;
        let pending = match pending {
            StreamPendingReply::Empty => 0,
            StreamPendingReply::Data(data) => data.count as u64,
        };
        let dead_letter: u64 = conn.xlen(&*self.cfg.dead_letter_key).await?;
        Ok(QueueStats {
            waiting: total.saturating_sub(pending),
            pending,
            dead_letter,
        })
    }

    #[must_use]
    pub fn cfg(&self) -> &QueueConfig {
        &self.cfg
    }
}

fn done_key(job_id: Uuid) -> String {
    format!("ytdl:job:done:{job_id}")
}

fn parse_entry(entry: redis::streams::StreamId, recovered: bool) -> Result<QueuedJob, QueueError> {
    let entry_id = entry.id;
    let map = entry.map;
    let payload: String = match map.get("data") {
        Some(value) => redis::from_redis_value(value)?,
        None => return Err(QueueError::MissingPayload { entry_id }),
    };
    Ok(QueuedJob {
        entry_id,
        recovered,
        job: serde_json::from_str(&payload)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        entities::{ChatConfig, JobTarget, Params},
        value_objects::MediaType,
    };
    use testcontainers_modules::testcontainers::{
        core::{IntoContainerPort as _, WaitFor},
        runners::AsyncRunner as _,
        ContainerAsync, GenericImage,
    };
    struct Fixture {
        queue: RedisJobQueue,
        _redis: ContainerAsync<GenericImage>,
    }
    impl Fixture {
        async fn new() -> Self {
            Self::with_config(0, 2).await
        }

        async fn with_config(claim_min_idle_ms: u64, max_attempts: u32) -> Self {
            let container = GenericImage::new("redis", "7-alpine")
                .with_exposed_port(6379.tcp())
                .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
                .start()
                .await
                .unwrap();
            let port = container.get_host_port_ipv4(6379).await.unwrap();
            let conn = ConnectionManager::new(redis::Client::open(format!("redis://127.0.0.1:{port}")).unwrap())
                .await
                .unwrap();
            let prefix = format!("test:{}", Uuid::now_v7());
            Self {
                queue: RedisJobQueue::new(
                    conn,
                    Arc::new(QueueConfig {
                        block_ms: 1,
                        stream_key: format!("{prefix}:stream").into_boxed_str(),
                        group: format!("{prefix}:workers").into_boxed_str(),
                        dead_letter_key: format!("{prefix}:dead").into_boxed_str(),
                        claim_min_idle_ms,
                        max_attempts,
                        ..QueueConfig::default()
                    }),
                ),
                _redis: container,
            }
        }
        async fn enqueue(&self) -> QueuedJob {
            let job = DownloadJob::new(
                MediaType::Video,
                Some("https://media.example.test/item".parse().unwrap()),
                Params::default(),
                ChatConfig::new(1, false, "en".into()),
                false,
                JobTarget::Command { chat_id: 1, message_id: 1 },
            );
            self.queue.enqueue(&job).await.unwrap();
            self.queue.ensure_group().await.unwrap();
            self.queue.read_next(&mut self.queue.conn.clone(), "first").await.unwrap().unwrap()
        }

        async fn enqueue_job(&self, job: &DownloadJob) -> QueuedJob {
            self.queue.enqueue(job).await.unwrap();
            self.queue.ensure_group().await.unwrap();
            self.queue.read_next(&mut self.queue.conn.clone(), "first").await.unwrap().unwrap()
        }
    }
    #[tokio::test]
    async fn reclaimed_entry_is_the_same_stream_entry_marked_for_uncertain_recovery() {
        let fixture = Fixture::new().await;
        let first = fixture.enqueue().await;
        assert!(!first.recovered);
        let reclaimed = fixture
            .queue
            .reclaim_stale("replacement", &mut "0-0".to_owned())
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(reclaimed.entry_id, first.entry_id);
        assert!(reclaimed.recovered);
    }

    #[tokio::test]
    async fn malformed_new_entry_is_removed_instead_of_staying_pending_until_reclaim() {
        let fixture = Fixture::new().await;
        let mut conn = fixture.queue.conn.clone();
        let _: String = redis::cmd("XADD")
            .arg(&*fixture.queue.cfg.stream_key)
            .arg("*")
            .arg("unexpected")
            .arg("field")
            .query_async(&mut conn)
            .await
            .unwrap();
        let _: String = redis::cmd("XADD")
            .arg(&*fixture.queue.cfg.stream_key)
            .arg("*")
            .arg("data")
            .arg("not valid json")
            .query_async(&mut conn)
            .await
            .unwrap();
        fixture.queue.ensure_group().await.unwrap();

        assert!(fixture.queue.read_next(&mut conn, "first").await.unwrap().is_none());
        assert!(fixture.queue.read_next(&mut conn, "first").await.unwrap().is_none());
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (0, 0));
    }

    #[tokio::test]
    async fn unresolved_entry_is_reclaimed_only_after_its_idle_threshold() {
        let fixture = Fixture::with_config(300, 2).await;
        let first = fixture.enqueue().await;
        assert!(fixture.queue.reclaim_stale("replacement", &mut "0-0".to_owned()).await.unwrap().is_empty());

        let reclaimed = eventually_reclaim(&fixture.queue, "replacement", std::time::Duration::from_secs(2)).await;
        assert_eq!(reclaimed.entry_id, first.entry_id);
        assert!(reclaimed.recovered);
    }

    #[tokio::test]
    async fn stale_consumer_cannot_ack_an_entry_after_reclaim() {
        let fixture = Fixture::new().await;
        let first = fixture.enqueue().await;
        let _ = fixture
            .queue
            .reclaim_stale("replacement", &mut "0-0".to_owned())
            .await
            .unwrap()
            .pop()
            .unwrap();

        assert!(!fixture.queue.ack(&first.entry_id, "first").await.unwrap());
        assert_eq!(fixture.queue.stats().await.unwrap().pending, 1);
    }

    #[tokio::test]
    async fn stale_consumer_cannot_retry_an_entry_after_reclaim() {
        let fixture = Fixture::new().await;
        let first = fixture.enqueue().await;
        let _ = fixture
            .queue
            .reclaim_stale("replacement", &mut "0-0".to_owned())
            .await
            .unwrap()
            .pop()
            .unwrap();
        let mut retry = first.job.clone();
        retry.attempts = 1;

        assert!(!fixture.queue.retry(&retry, &first.entry_id, "first").await.unwrap());
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (0, 1));
    }

    #[tokio::test]
    async fn stale_consumer_cannot_dead_letter_an_entry_after_reclaim() {
        let fixture = Fixture::new().await;
        let first = fixture.enqueue().await;
        let _ = fixture
            .queue
            .reclaim_stale("replacement", &mut "0-0".to_owned())
            .await
            .unwrap()
            .pop()
            .unwrap();

        assert!(!fixture.queue.dead_letter(&first.job, &first.entry_id, "first").await.unwrap());
        assert_eq!(fixture.queue.stats().await.unwrap().dead_letter, 0);
    }
    #[tokio::test]
    async fn ack_removes_completed_work_and_done_marker_suppresses_replay() {
        let fixture = Fixture::new().await;
        let queued = fixture.enqueue().await;
        fixture.queue.mark_done(queued.job.job_id).await.unwrap();
        assert!(fixture.queue.is_done(queued.job.job_id).await.unwrap());
        assert!(fixture.queue.ack(&queued.entry_id, "first").await.unwrap());
        assert!(!fixture.queue.ack(&queued.entry_id, "first").await.unwrap());
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (0, 0));
        let stream_len: u64 = fixture.queue.conn.clone().xlen(&*fixture.queue.cfg.stream_key).await.unwrap();
        assert_eq!(stream_len, 0);
    }

    #[tokio::test]
    async fn statistics_separate_waiting_pending_completed_and_dead_letter_entries() {
        let fixture = Fixture::new().await;
        let make_job = |message_id| {
            DownloadJob::new(
                MediaType::Video,
                Some(format!("https://media.example.test/{message_id}").parse().unwrap()),
                Params::default(),
                ChatConfig::new(1, false, "en".into()),
                false,
                JobTarget::Command { chat_id: 1, message_id },
            )
        };
        let pending = make_job(1);
        let other_pending = make_job(2);
        let completed = make_job(3);
        let failed = make_job(4);
        let waiting = make_job(5);
        for job in [&pending, &other_pending, &completed, &failed, &waiting] {
            fixture.queue.enqueue(job).await.unwrap();
        }
        fixture.queue.ensure_group().await.unwrap();
        let mut conn = fixture.queue.conn.clone();
        let first = fixture.queue.read_next(&mut conn, "first").await.unwrap().unwrap();
        let second = fixture.queue.read_next(&mut conn, "second").await.unwrap().unwrap();
        let third = fixture.queue.read_next(&mut conn, "third").await.unwrap().unwrap();
        let fourth = fixture.queue.read_next(&mut conn, "fourth").await.unwrap().unwrap();

        assert!(fixture.queue.ack(&third.entry_id, "third").await.unwrap());
        assert!(fixture.queue.dead_letter(&fourth.job, &fourth.entry_id, "fourth").await.unwrap());
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending, stats.dead_letter), (1, 2, 1));

        assert!(fixture.queue.ack(&first.entry_id, "first").await.unwrap());
        assert!(fixture.queue.ack(&second.entry_id, "second").await.unwrap());
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending, stats.dead_letter), (1, 0, 1));
    }

    #[tokio::test]
    async fn dead_letter_acknowledges_only_after_the_copy_is_written() {
        let fixture = Fixture::new().await;
        let queued = fixture.enqueue().await;
        assert!(fixture.queue.dead_letter(&queued.job, &queued.entry_id, "first").await.unwrap());
        // Replaying after the Redis reply is notionally lost cannot put the source back into
        // normal execution; the committed ACK makes ownership validation fail before XADD.
        assert!(!fixture.queue.dead_letter(&queued.job, &queued.entry_id, "first").await.unwrap());
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending, stats.dead_letter), (0, 0, 1));
    }

    #[tokio::test]
    async fn terminal_retry_replaces_the_pending_entry_immediately() {
        let fixture = Fixture::new().await;
        let queued = fixture.enqueue().await;
        let mut retry = queued.job.clone();
        retry.attempts = 1;
        assert!(fixture.queue.retry(&retry, &queued.entry_id, "first").await.unwrap());
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (1, 0));
        let retried = fixture
            .queue
            .read_next(&mut fixture.queue.conn.clone(), "retry")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retried.job.job_id, queued.job.job_id);
        assert_eq!(retried.job.attempts, 1);
        assert_ne!(retried.entry_id, queued.entry_id);
    }

    #[tokio::test]
    async fn retry_transition_is_a_one_shot_for_its_pending_owner() {
        let fixture = Fixture::new().await;
        let queued = fixture.enqueue().await;
        let mut retry = queued.job.clone();
        retry.attempts = 1;

        assert!(fixture.queue.retry(&retry, &queued.entry_id, "first").await.unwrap());
        assert!(!fixture.queue.retry(&retry, &queued.entry_id, "first").await.unwrap());

        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (1, 0));
    }

    #[tokio::test]
    async fn concurrent_terminal_retries_create_only_one_replacement() {
        let fixture = Fixture::new().await;
        let queued = fixture.enqueue().await;
        let mut retry = queued.job.clone();
        retry.attempts = 1;
        let (first, second) = tokio::join!(
            fixture.queue.retry(&retry, &queued.entry_id, "first"),
            fixture.queue.retry(&retry, &queued.entry_id, "first"),
        );

        assert_eq!(usize::from(first.unwrap()) + usize::from(second.unwrap()), 1);
        let stats = fixture.queue.stats().await.unwrap();
        let stream_len: u64 = fixture.queue.conn.clone().xlen(&*fixture.queue.cfg.stream_key).await.unwrap();
        assert_eq!((stats.waiting, stats.pending, stream_len), (1, 0, 1));
    }

    #[tokio::test]
    async fn redis_redelivery_count_does_not_change_application_attempts() {
        let fixture = Fixture::new().await;
        let job = DownloadJob::new(
            MediaType::Video,
            Some("https://media.example.test/item".parse().unwrap()),
            Params::default(),
            ChatConfig::new(1, false, "en".into()),
            false,
            JobTarget::Command { chat_id: 1, message_id: 1 },
        );
        let first = fixture.enqueue_job(&job).await;
        let recovered = fixture
            .queue
            .reclaim_stale("second", &mut "0-0".to_owned())
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(recovered.job.attempts, 0);
        assert!(recovered.recovered);

        let pending: Vec<(String, String, u64, u64)> = redis::cmd("XPENDING")
            .arg(&*fixture.queue.cfg.stream_key)
            .arg(&*fixture.queue.cfg.group)
            .arg(&first.entry_id)
            .arg(&first.entry_id)
            .arg(1)
            .query_async(&mut fixture.queue.conn.clone())
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1, "second");
        assert!(pending[0].3 >= 2, "Redis delivery count should reflect the reclaim");
        assert_eq!(recovered.job.attempts, 0, "application attempts come from the serialized job");
    }

    #[tokio::test]
    async fn malformed_reclaimed_entry_is_removed_from_the_pel_and_stream() {
        let fixture = Fixture::new().await;
        let mut conn = fixture.queue.conn.clone();
        let _: String = redis::cmd("XADD")
            .arg(&*fixture.queue.cfg.stream_key)
            .arg("*")
            .arg("data")
            .arg("not valid json")
            .query_async(&mut conn)
            .await
            .unwrap();
        fixture.queue.ensure_group().await.unwrap();
        let _: redis::streams::StreamReadReply = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(&*fixture.queue.cfg.group)
            .arg("old-owner")
            .arg("STREAMS")
            .arg(&*fixture.queue.cfg.stream_key)
            .arg(">")
            .query_async(&mut conn)
            .await
            .unwrap();

        let reclaimed = fixture
            .queue
            .reclaim_stale("new-owner", &mut "0-0".to_owned())
            .await
            .unwrap();
        assert!(reclaimed.is_empty());
        let stats = fixture.queue.stats().await.unwrap();
        let stream_len: u64 = fixture.queue.conn.clone().xlen(&*fixture.queue.cfg.stream_key).await.unwrap();
        assert_eq!((stats.waiting, stats.pending, stream_len), (0, 0, 0));
    }

    async fn eventually_reclaim(queue: &RedisJobQueue, consumer: &str, timeout: std::time::Duration) -> QueuedJob {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let mut cursor = "0-0".to_owned();
            if let Some(job) = queue.reclaim_stale(consumer, &mut cursor).await.unwrap().pop() {
                return job;
            }
            assert!(tokio::time::Instant::now() < deadline, "entry did not become reclaimable before timeout");
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}
