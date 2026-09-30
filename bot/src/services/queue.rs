//! Durable download queue backed by a Redis Stream + consumer group.
//!
//! Known outcomes finish their entry. Abandoned deliveries are discarded, never re-executed.

use std::{future::Future, sync::Arc};

use redis::{
    aio::ConnectionManager,
    streams::{StreamAutoClaimReply, StreamPendingReply, StreamReadReply},
    AsyncCommands as _, RedisError,
};
use tracing::error;
use uuid::Uuid;

use crate::{config::QueueConfig, entities::DownloadJob};

const ACK_DELETE: &str = r"
local pending = redis.call('XPENDING', KEYS[1], ARGV[1], ARGV[2], ARGV[2], 1, ARGV[3])
if #pending == 0 then return 0 end
local acked = redis.call('XACK', KEYS[1], ARGV[1], ARGV[2])
if acked == 1 then redis.call('XDEL', KEYS[1], ARGV[2]) end
return acked
";

// Only active work or a known pre-admission wait refreshes idle time; an old
// consumer cannot take an entry back from the cleanup consumer.
const REFRESH_LIVENESS: &str = r"
local pending = redis.call('XPENDING', KEYS[1], ARGV[1], ARGV[2], ARGV[2], 1, ARGV[3])
if #pending == 0 then return 0 end
redis.call('XCLAIM', KEYS[1], ARGV[1], ARGV[3], 0, ARGV[2], 'JUSTID')
return 1
";

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error(transparent)]
    Redis(#[from] RedisError),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("Stream entry {entry_id} is missing the `data` field")]
    MissingPayload { entry_id: String },
}

/// A job read off the stream together with its entry id.
pub struct QueuedJob {
    pub entry_id: String,
    pub job: DownloadJob,
}

/// A point-in-time snapshot of the queue, for `/stats`.
#[derive(Debug, Default, Clone, Copy)]
pub struct QueueStats {
    pub waiting: u64,
    /// Delivered but not acknowledged. This can include work awaiting stale cleanup.
    pub pending: u64,
}

pub struct RedisJobQueue {
    conn: ConnectionManager,
    cfg: Arc<QueueConfig>,
}

/// Guest queries allow a single reply. Reserve before contacting Telegram so a repeated update
/// cannot start a second reply or download, including after an ambiguous request failure.
pub trait GuestQueuePort: Send + Sync {
    fn reserve_guest_query(&self, query_id: &str) -> impl Future<Output = Result<bool, QueueError>> + Send;
    fn enqueue_guest(&self, job: &DownloadJob) -> impl Future<Output = Result<(), QueueError>> + Send;
}

impl GuestQueuePort for RedisJobQueue {
    async fn reserve_guest_query(&self, query_id: &str) -> Result<bool, QueueError> {
        let mut conn = self.conn.clone();
        let reserved: Option<String> = redis::cmd("SET")
            .arg(format!("{}:guest:{query_id}", self.cfg.stream_key))
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(86_400)
            .query_async(&mut conn)
            .await?;
        Ok(reserved.is_some())
    }

    async fn enqueue_guest(&self, job: &DownloadJob) -> Result<(), QueueError> {
        self.enqueue(job).await
    }
}

impl RedisJobQueue {
    pub async fn refresh_liveness(&self, entry_id: &str, consumer: &str) -> Result<bool, QueueError> {
        let mut conn = self.conn.clone();
        let result: i64 = redis::cmd("EVAL")
            .arg(REFRESH_LIVENESS)
            .arg(1)
            .arg(&*self.cfg.stream_key)
            .arg(&*self.cfg.group)
            .arg(entry_id)
            .arg(consumer)
            .query_async(&mut conn)
            .await?;
        Ok(result == 1)
    }
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
        let mut conn = self.conn.clone();
        let _: String = redis::cmd("XADD")
            .arg(&*self.cfg.stream_key)
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
        match parse_entry(entry) {
            Ok(job) => Ok(Some(job)),
            Err(err) => {
                error!(%err, entry_id, "Dropping unparseable queued job");
                let _ = self.ack(&entry_id, consumer).await?;
                Ok(None)
            }
        }
    }

    /// Claims and discards deliveries abandoned beyond `claim_min_idle_ms`.
    pub async fn cleanup_stale(&self, consumer: &str, cursor: &mut String) -> Result<usize, QueueError> {
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
        let mut cleaned = 0;
        for entry in reply.claimed {
            let id = entry.id.clone();
            let parsed = parse_entry(entry);
            if self.ack(&id, consumer).await? {
                match parsed {
                    Ok(job) => {
                        error!(job_id = %job.job.job_id, entry_id = %id, cleanup_consumer = consumer, idle_ms_at_least = self.cfg.claim_min_idle_ms, "Discarded abandoned download job without re-execution")
                    }
                    Err(err) => {
                        error!(%err, entry_id = %id, cleanup_consumer = consumer, idle_ms_at_least = self.cfg.claim_min_idle_ms, "Discarded malformed abandoned download job")
                    }
                }
                cleaned += 1;
            }
        }
        Ok(cleaned)
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

    /// True if this `job_id` was already completed (best-effort replay deduplication).
    pub async fn is_done(&self, job_id: Uuid) -> Result<bool, QueueError> {
        Ok(self.conn.clone().exists(done_key(job_id)).await?)
    }

    /// Marks a job as finished before acknowledgement so a lost ACK remains identifiable.
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
        Ok(QueueStats {
            waiting: total.saturating_sub(pending),
            pending,
        })
    }
}

fn done_key(job_id: Uuid) -> String {
    format!("ytdl:job:done:{job_id}")
}

fn parse_entry(entry: redis::streams::StreamId) -> Result<QueuedJob, QueueError> {
    let entry_id = entry.id;
    let map = entry.map;
    let payload: String = match map.get("data") {
        Some(value) => redis::from_redis_value(value)?,
        None => return Err(QueueError::MissingPayload { entry_id }),
    };
    Ok(QueuedJob {
        entry_id,
        job: serde_json::from_str(&payload)?,
    })
}

#[cfg(test)]
pub mod test_support {
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
    pub(crate) struct Fixture {
        pub(crate) queue: Arc<RedisJobQueue>,
        _redis: ContainerAsync<GenericImage>,
    }
    impl Fixture {
        pub(crate) async fn new() -> Self {
            Self::with_config(0).await
        }

        pub(crate) async fn with_config(claim_min_idle_ms: u64) -> Self {
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
                queue: Arc::new(RedisJobQueue::new(
                    conn,
                    Arc::new(QueueConfig {
                        block_ms: 1,
                        stream_key: format!("{prefix}:stream").into_boxed_str(),
                        group: format!("{prefix}:workers").into_boxed_str(),
                        claim_min_idle_ms,
                        ..QueueConfig::default()
                    }),
                )),
                _redis: container,
            }
        }
        pub(crate) async fn enqueue(&self) -> QueuedJob {
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

        pub(crate) fn connection(&self) -> ConnectionManager {
            self.queue.conn.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{test_support::Fixture, *};
    use crate::{
        entities::{ChatConfig, JobTarget, Params},
        value_objects::MediaType,
    };

    #[tokio::test]
    async fn guest_reservation_is_atomic_and_has_ttl() {
        let fixture = Fixture::new().await;
        let (first, second) = tokio::join!(
            fixture.queue.reserve_guest_query("synthetic-query"),
            fixture.queue.reserve_guest_query("synthetic-query"),
        );
        assert_ne!(first.unwrap(), second.unwrap());
        let mut conn = fixture.queue.conn.clone();
        let key = format!("{}:guest:synthetic-query", fixture.queue.cfg.stream_key);
        let ttl: i64 = redis::cmd("TTL").arg(&key).query_async(&mut conn).await.unwrap();
        assert!((1..=86_400).contains(&ttl));
        let _: i64 = redis::cmd("DEL").arg(key).query_async(&mut conn).await.unwrap();
        assert!(fixture.queue.reserve_guest_query("synthetic-query").await.unwrap());
    }

    #[tokio::test]
    async fn abandoned_delivery_is_discarded_without_reexecution() {
        let fixture = Fixture::new().await;
        let first = fixture.enqueue().await;
        assert_eq!(fixture.queue.cleanup_stale("cleanup", &mut "0-0".to_owned()).await.unwrap(), 1);
        assert!(!fixture.queue.ack(&first.entry_id, "first").await.unwrap());
        assert!(fixture.queue.read_next(&mut fixture.connection(), "next").await.unwrap().is_none());
        assert_eq!(
            (
                fixture.queue.stats().await.unwrap().waiting,
                fixture.queue.stats().await.unwrap().pending
            ),
            (0, 0)
        );
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
    async fn unresolved_entry_is_discarded_only_after_its_idle_threshold() {
        let fixture = Fixture::with_config(300).await;
        let first = fixture.enqueue().await;
        assert_eq!(fixture.queue.cleanup_stale("cleanup", &mut "0-0".to_owned()).await.unwrap(), 0);

        eventually_cleanup(&fixture.queue, "cleanup", std::time::Duration::from_secs(2)).await;
        assert!(!fixture.queue.ack(&first.entry_id, "first").await.unwrap());
        assert_eq!(fixture.queue.stats().await.unwrap().pending, 0);
    }

    #[tokio::test]
    async fn stale_consumer_cannot_ack_an_entry_after_cleanup() {
        let fixture = Fixture::new().await;
        let first = fixture.enqueue().await;
        let _ = fixture.queue.cleanup_stale("cleanup", &mut "0-0".to_owned()).await.unwrap();

        assert!(!fixture.queue.ack(&first.entry_id, "first").await.unwrap());
        assert_eq!(fixture.queue.stats().await.unwrap().pending, 0);
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
    async fn statistics_separate_waiting_pending_and_finalized_entries() {
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
        assert!(fixture.queue.ack(&fourth.entry_id, "fourth").await.unwrap());
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (1, 2));

        assert!(fixture.queue.ack(&first.entry_id, "first").await.unwrap());
        assert!(fixture.queue.ack(&second.entry_id, "second").await.unwrap());
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (1, 0));
    }

    #[tokio::test]
    async fn malformed_abandoned_entry_is_removed_from_the_pel_and_stream() {
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

        assert_eq!(fixture.queue.cleanup_stale("cleanup", &mut "0-0".to_owned()).await.unwrap(), 1);
        let stats = fixture.queue.stats().await.unwrap();
        let stream_len: u64 = fixture.queue.conn.clone().xlen(&*fixture.queue.cfg.stream_key).await.unwrap();
        assert_eq!((stats.waiting, stats.pending, stream_len), (0, 0, 0));
    }

    async fn eventually_cleanup(queue: &RedisJobQueue, consumer: &str, timeout: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let mut cursor = "0-0".to_owned();
            if queue.cleanup_stale(consumer, &mut cursor).await.unwrap() > 0 {
                return;
            }
            assert!(tokio::time::Instant::now() < deadline, "entry was not cleaned before timeout");
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}
