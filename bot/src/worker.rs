//! Download worker pool draining the Redis queue.
//!
//! Each worker is a distinct stream consumer that blocks for the next [`DownloadJob`], runs the
//! real download interactor inside a fresh request scope (mirroring what the telers integration
//! does per update), then acks. Jobs left pending by a crashed worker are recovered continuously via
//! [`RedisJobQueue::reclaim_stale`]. Concurrency is bounded by the number of workers, which is the
//! cap that smooths the post-restart backlog burst.

use std::{panic::AssertUnwindSafe, sync::Arc, time::Duration};

use downloader_client::outcome::track_execution;
use froodi::{async_impl::Container, DefaultScope::Request, ResolveErrorKind, ScopeWithErrorKind};
use futures_util::FutureExt as _;
use redis::aio::ConnectionManager;
use telers::{errors::HandlerError, methods::SetMessageReaction, Bot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::{
    config::{DomainsWithReactionsConfig, TimeoutsConfig},
    entities::{DownloadJob, JobTarget},
    interactors::{audio, auto, chosen_inline, photo, video, Interactor as _},
    services::{
        messenger::telegram::TelegramMessenger,
        queue::{QueueError, QueuedJob, RedisJobQueue},
    },
    value_objects::MediaType,
};

const ALL_NODES_BUSY_RETRY_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
enum JobError {
    #[error(transparent)]
    Scope(#[from] ScopeWithErrorKind),
    #[error(transparent)]
    Resolve(#[from] ResolveErrorKind),
    #[error(transparent)]
    Handler(#[from] HandlerError),
    #[error("Command job is missing its URL")]
    MissingUrl,
    #[error("Job timed out")]
    Timeout,
}

enum JobFailure {
    Terminal,
    Uncertain,
    Permanent,
}

impl JobError {
    fn failure(&self) -> JobFailure {
        match self {
            Self::MissingUrl => JobFailure::Permanent,
            Self::Timeout => JobFailure::Uncertain,
            Self::Scope(_) | Self::Resolve(_) | Self::Handler(_) => JobFailure::Terminal,
        }
    }
}

/// Ensures the consumer group exists and spawns `workers` worker tasks. Returns their join handles
/// so the caller can await cancellation after signalling `shutdown`. Pending work
/// stays in the stream and is recovered by normal `XAUTOCLAIM` delivery.
pub async fn spawn_pool(container: Container, shutdown: CancellationToken, workers: usize) -> Vec<JoinHandle<()>> {
    let queue = match container.get::<RedisJobQueue>().await {
        Ok(queue) => queue,
        Err(err) => {
            error!(%err, "Resolve job queue error; download workers not started");
            return Vec::new();
        }
    };
    if let Err(err) = queue.ensure_group().await {
        error!(%err, "Create consumer group error; download workers not started");
        return Vec::new();
    }
    info!(workers, "Starting download workers");
    (0..workers)
        .map(|id| {
            let consumer = format!("worker-{}-{id}", Uuid::now_v7());
            let container = container.clone();
            let queue = queue.clone();
            let shutdown = shutdown.clone();
            tokio::spawn(async move { worker_loop(consumer, container, queue, shutdown).await })
        })
        .collect()
}

#[instrument(skip_all, fields(consumer))]
async fn worker_loop(consumer: String, container: Container, queue: Arc<RedisJobQueue>, shutdown: CancellationToken) {
    let mut read_conn = match container.get_transient::<ConnectionManager>().await {
        Ok(conn) => conn,
        Err(err) => {
            error!(%err, "Resolve read connection error; worker not started");
            return;
        }
    };

    let mut reclaim_cursor = "0-0".to_owned();
    loop {
        if shutdown.is_cancelled() {
            break;
        }
        // Scan continuously, one entry at a time. Startup-only reclaim loses jobs
        // that have not reached min-idle yet, and batches can expire while queued.
        let reclaimed = tokio::select! {
            () = shutdown.cancelled() => break,
            reclaimed = queue.reclaim_stale(&consumer, &mut reclaim_cursor) => reclaimed,
        };
        match reclaimed {
            Ok(reclaimed) => {
                for queued in reclaimed {
                    process(container.clone(), &queue, &consumer, queued).await;
                }
            }
            Err(err) => error!(%err, "Reclaim stale jobs error"),
        }
        tokio::select! {
            () = shutdown.cancelled() => break,
            res = queue.read_next(&mut read_conn, &consumer) => match res {
                Ok(Some(queued)) => {
                    process(container.clone(), &queue, &consumer, queued).await;
                },
                Ok(None) => {}
                Err(err) => {
                    error!(%err, "Read job error");
                    // back off briefly so a persistent Redis error doesn't hot-loop
                    tokio::select! {
                        () = shutdown.cancelled() => break,
                        () = tokio::time::sleep(Duration::from_secs(1)) => {}
                    }
                }
            },
        }
    }
    info!("Download worker stopped");
}

#[instrument(skip_all, fields(job_id = %queued.job.job_id, entry_id = queued.entry_id))]
async fn process(container: Container, queue: &RedisJobQueue, consumer: &str, queued: QueuedJob) {
    // Best-effort dedup: a job replayed after a crash-before-ack must not be delivered twice.
    match queue.is_done(queued.job.job_id).await {
        Ok(true) => {
            if let Err(err) = queue.ack(&queued.entry_id, consumer).await {
                warn!(%err, "Ack deduplicated job error");
            }
            return;
        }
        Ok(false) => {}
        Err(err) => {
            warn!(%err, "Dedup check failed; leaving job pending");
            return;
        }
    }
    if queued.recovered {
        // The previous owner stopped without proving that its execution ended.
        // Count that uncertain attempt before making a replacement executable.
        retry_or_dead_letter(queue, &queued.job, &queued.entry_id, consumer, "Recovered uncertain job").await;
        return;
    }
    info!("Processing download job");
    let job = run_job(container, &queued.job);
    let (result, execution_outcome) = track_execution(AssertUnwindSafe(job).catch_unwind()).await;
    if execution_outcome.is_uncertain() {
        match result {
            Ok(Ok(())) => warn!("Job returned successfully after an uncertain downloader outcome; leaving entry pending"),
            Ok(Err(err)) => warn!(%err, "Job outcome is uncertain; leaving entry pending for recovery"),
            Err(_) => error!("Job panicked after an uncertain downloader outcome; leaving entry pending for recovery"),
        }
        return;
    }
    // Playlist interactors can report item errors while still successfully sending other items;
    // only retry capacity pressure when the whole worker operation failed.
    if execution_outcome.all_nodes_busy()
        && !execution_outcome.execution_started()
        && matches!(&result, Ok(Err(_)))
    {
        warn!("All downloader nodes rejected admission; retrying without consuming an attempt");
        match retry_without_consuming_attempt(&queue, &queued.job, &queued.entry_id, consumer).await {
            Ok(true) => {}
            Ok(false) => warn!("Capacity retry entry was already transitioned or reclaimed"),
            Err(err) => error!(%err, "Capacity retry job error"),
        }
        return;
    }
    match result {
        Ok(Ok(())) => {
            if let Err(err) = queue.mark_done(queued.job.job_id).await {
                error!(%err, "Mark job done error");
                return;
            }
            match queue.ack(&queued.entry_id, consumer).await {
                Ok(true) => {}
                Ok(false) => warn!("Completed job is no longer owned by this consumer; leaving the current owner untouched"),
                Err(err) => error!(%err, "Ack job error"),
            }
        }
        Ok(Err(err)) => match err.failure() {
            JobFailure::Terminal => {
                warn!(%err, "Job failed terminally; retrying");
                retry_or_dead_letter(queue, &queued.job, &queued.entry_id, consumer, "Terminal job failure").await;
            }
            JobFailure::Permanent => {
                warn!(%err, "Job is permanently invalid; dead-lettering");
                match queue.dead_letter(&queued.job, &queued.entry_id, consumer).await {
                    Ok(true) => {}
                    Ok(false) => warn!("Dead-letter entry is no longer owned by this consumer"),
                    Err(err) => error!(%err, "Dead-letter job error"),
                }
            }
            JobFailure::Uncertain => warn!(%err, "Job outcome is uncertain; leaving entry pending for recovery"),
        },
        Err(_) => error!("Job panicked; leaving entry pending for recovery"),
    }
}

async fn retry_or_dead_letter(queue: &RedisJobQueue, job: &DownloadJob, entry_id: &str, consumer: &str, reason: &str) {
    let mut retry = job.clone();
    retry.attempts = retry.attempts.saturating_add(1);
    if retry.attempts >= queue.cfg().max_attempts {
        warn!(attempts = retry.attempts, %reason, "Job exceeded max attempts; dead-lettering");
        match queue.dead_letter(&retry, entry_id, consumer).await {
            Ok(true) => {}
            Ok(false) => warn!(%reason, "Dead-letter entry was already transitioned or reclaimed"),
            Err(err) => error!(%err, "Dead-letter job error"),
        }
    } else {
        match queue.retry(&retry, entry_id, consumer).await {
            Ok(true) => {}
            Ok(false) => warn!(%reason, "Retry entry was already transitioned or reclaimed"),
            Err(err) => error!(%err, "Retry job error"),
        }
    }
}

async fn retry_without_consuming_attempt(
    queue: &RedisJobQueue,
    job: &DownloadJob,
    entry_id: &str,
    consumer: &str,
) -> Result<bool, QueueError> {
    tokio::time::sleep(ALL_NODES_BUSY_RETRY_DELAY).await;
    queue.retry(job, entry_id, consumer).await
}

async fn run_job(container: Container, job: &DownloadJob) -> Result<(), JobError> {
    let child = container.enter().with_scope(Request).build()?;
    let job_timeout = Duration::from_secs_f32(child.get::<TimeoutsConfig>().await.unwrap().job);
    let result = match tokio::time::timeout(job_timeout, Box::pin(run_in_scope(&child, job))).await {
        Ok(result) => result,
        Err(_) => Err(JobError::Timeout),
    };
    // Clear the acknowledgment reaction once processing is done, on success or failure alike (the
    // handler only enqueued). No-op for inline jobs and links the reaction middleware ignored.
    if let JobTarget::Command { chat_id, message_id } = &job.target {
        let _ = tokio::time::timeout(Duration::from_secs(5), clear_reaction(&child, job, *chat_id, *message_id)).await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), child.close()).await;
    result
}

async fn run_in_scope(child: &Container, job: &DownloadJob) -> Result<(), JobError> {
    match &job.target {
        JobTarget::Command { chat_id, message_id } => {
            let url = job.url.as_ref().ok_or(JobError::MissingUrl)?;
            if job.auto {
                // Bare link with no committed type: classify (video -> audio -> photo) and download.
                if job.quiet {
                    let interactor = child.get::<auto::AutoQuiet<TelegramMessenger>>().await?;
                    interactor
                        .execute(auto::AutoQuietInput {
                            message_id: *message_id,
                            chat_id: *chat_id,
                            params: &job.params,
                            url,
                            link_is_visible: job.link_is_visible,
                        })
                        .await?;
                } else {
                    let interactor = child.get::<auto::Auto<TelegramMessenger>>().await?;
                    Box::pin(interactor.execute(auto::AutoInput {
                        message_id: *message_id,
                        chat_id: *chat_id,
                        url,
                        params: &job.params,
                        chat_cfg: &job.chat_cfg,
                        link_is_visible: job.link_is_visible,
                    }))
                    .await?;
                }
            } else {
                // Each media type has its own interactor + `Input` type (same fields, distinct types),
                // so these can't collapse into a macro the way the inline arm below does.
                match job.media_type {
                    MediaType::Video => {
                        let interactor = child.get::<video::Download<TelegramMessenger>>().await?;
                        Box::pin(interactor.execute(video::DownloadInput {
                            message_id: *message_id,
                            chat_id: *chat_id,
                            params: &job.params,
                            url,
                            chat_cfg: &job.chat_cfg,
                            link_is_visible: job.link_is_visible,
                            prefetched: None,
                        }))
                        .await?;
                    }
                    MediaType::Audio => {
                        let interactor = child.get::<audio::Download<TelegramMessenger>>().await?;
                        Box::pin(interactor.execute(audio::DownloadInput {
                            message_id: *message_id,
                            chat_id: *chat_id,
                            params: &job.params,
                            url,
                            chat_cfg: &job.chat_cfg,
                            link_is_visible: job.link_is_visible,
                            progress_message_id: job.progress_message_id,
                            base_text: job.base_text.as_deref(),
                            prefetched: None,
                        }))
                        .await?;
                    }
                    MediaType::Photo => {
                        let interactor = child.get::<photo::Download<TelegramMessenger>>().await?;
                        interactor
                            .execute(photo::DownloadInput {
                                message_id: *message_id,
                                chat_id: *chat_id,
                                params: &job.params,
                                url,
                                chat_cfg: &job.chat_cfg,
                                link_is_visible: job.link_is_visible,
                                prefetched: None,
                            })
                            .await?;
                    }
                }
            }
        }
        JobTarget::Inline {
            inline_message_id,
            result_id,
        } => {
            let url = job.url.as_ref();

            macro_rules! run {
                ($download:ty) => {{
                    let interactor = child.get::<$download>().await?;
                    interactor
                        .execute(chosen_inline::DownloadInput {
                            params: &job.params,
                            url,
                            chat_cfg: &job.chat_cfg,
                            link_is_visible: job.link_is_visible,
                            inline_message_id,
                            result_id,
                            prefetched: None,
                        })
                        .await?;
                }};
            }
            if job.auto {
                run!(chosen_inline::DownloadAuto<TelegramMessenger>);
            } else {
                match job.media_type {
                    MediaType::Video => run!(chosen_inline::DownloadVideo<TelegramMessenger>),
                    MediaType::Audio => run!(chosen_inline::DownloadAudio<TelegramMessenger>),
                    MediaType::Photo => run!(chosen_inline::DownloadPhoto<TelegramMessenger>),
                }
            }
        }
    }
    Ok(())
}

// Removes the acknowledgment reaction the middleware set on the user's message. The middleware sets
// it on receipt; the worker clears it here once the job is actually done (not when the handler
// finished enqueuing). No-op for links the reaction middleware ignores.
async fn clear_reaction(child: &Container, job: &DownloadJob, chat_id: i64, message_id: i64) {
    let Some(domain) = job.url.as_ref().and_then(|url| url.domain()) else {
        return;
    };
    let Ok(domains_with_reactions) = child.get::<DomainsWithReactionsConfig>().await else {
        return;
    };
    if !domains_with_reactions
        .domains
        .contains(&domain.trim_start_matches("www.").to_owned())
    {
        return;
    }
    let Ok(bot) = child.get::<Bot>().await else {
        return;
    };
    if let Err(err) = bot.send(SetMessageReaction::new(chat_id, message_id)).await {
        error!(%err, "Unset reaction error");
    }
}

#[cfg(test)]
mod tests {
    use redis::aio::ConnectionManager;
    use testcontainers_modules::testcontainers::{
        core::{IntoContainerPort as _, WaitFor},
        runners::AsyncRunner as _,
        ContainerAsync, GenericImage,
    };

    use super::*;
    use crate::{
        config::QueueConfig,
        entities::{ChatConfig, JobTarget, Params},
        value_objects::MediaType,
    };

    struct Fixture {
        queue: RedisJobQueue,
        conn: ConnectionManager,
        _redis: ContainerAsync<GenericImage>,
    }

    impl Fixture {
        async fn new(max_attempts: u32) -> Self {
            let redis = GenericImage::new("redis", "7-alpine")
                .with_exposed_port(6379.tcp())
                .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
                .start()
                .await
                .unwrap();
            let port = redis.get_host_port_ipv4(6379).await.unwrap();
            let conn = ConnectionManager::new(redis::Client::open(format!("redis://127.0.0.1:{port}")).unwrap())
                .await
                .unwrap();
            let queue_conn = conn.clone();
            let prefix = format!("test:{}", Uuid::now_v7());
            Self {
                queue: RedisJobQueue::new(
                    queue_conn,
                    Arc::new(QueueConfig {
                        block_ms: 1,
                        stream_key: format!("{prefix}:stream").into_boxed_str(),
                        group: format!("{prefix}:workers").into_boxed_str(),
                        dead_letter_key: format!("{prefix}:dead").into_boxed_str(),
                        claim_min_idle_ms: 0,
                        max_attempts,
                        ..QueueConfig::default()
                    }),
                ),
                conn,
                _redis: redis,
            }
        }

        async fn enqueue(&self, job: &DownloadJob, consumer: &str) -> QueuedJob {
            self.queue.ensure_group().await.unwrap();
            self.queue.enqueue(job).await.unwrap();
            self.queue.read_next(&mut self.conn.clone(), consumer).await.unwrap().unwrap()
        }
    }

    fn job() -> DownloadJob {
        DownloadJob::new(
            MediaType::Video,
            Some("https://media.example.test/item".parse().unwrap()),
            Params::default(),
            ChatConfig::new(1, false, "en".into()),
            false,
            JobTarget::Command { chat_id: 1, message_id: 1 },
        )
    }

    #[tokio::test]
    async fn max_attempts_allows_exactly_that_many_execution_opportunities() {
        use redis::streams::StreamRangeReply;
        let fixture = Fixture::new(2).await;
        let mut read_conn = fixture.conn.clone();
        let first = fixture.enqueue(&job(), "first").await;
        retry_or_dead_letter(&fixture.queue, &first.job, &first.entry_id, "first", "synthetic failure").await;

        let second = fixture
            .queue
            .read_next(&mut read_conn, "second")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.job.attempts, 1);
        retry_or_dead_letter(&fixture.queue, &second.job, &second.entry_id, "second", "synthetic failure").await;

        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending, stats.dead_letter), (0, 0, 1));
        let dead_letter: StreamRangeReply = redis::cmd("XRANGE")
            .arg(&*fixture.queue.cfg().dead_letter_key)
            .arg("-")
            .arg("+")
            .query_async(&mut read_conn)
            .await
            .unwrap();
        let payload = dead_letter.ids[0].get::<String>("data").unwrap();
        let dead_letter_job: DownloadJob = serde_json::from_str(&payload).unwrap();
        assert_eq!(dead_letter_job.attempts, 2);
    }

    #[tokio::test]
    async fn all_nodes_busy_retries_without_incrementing_attempts_or_dead_lettering() {
        let fixture = Fixture::new(2).await;
        let mut original = job();
        original.attempts = 1;
        let mut queued = fixture.enqueue(&original, "first").await;
        let mut consumer = "first";
        for next_consumer in ["second", "third"] {
            assert!(retry_without_consuming_attempt(&fixture.queue, &queued.job, &queued.entry_id, consumer)
                .await
                .unwrap());
            queued = fixture
                .queue
                .read_next(&mut fixture.conn.clone(), next_consumer)
                .await
                .unwrap()
                .unwrap();
            consumer = next_consumer;
            assert_eq!(queued.job.attempts, 1);
            assert_eq!(queued.job.job_id, original.job_id);
        }
        assert!(fixture.queue.ack(&queued.entry_id, consumer).await.unwrap());
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending, stats.dead_letter), (0, 0, 0));
    }

    #[tokio::test]
    async fn reclaimed_uncertain_job_consumes_one_attempt_before_becoming_executable() {
        let fixture = Fixture::new(4).await;
        let mut original = job();
        original.attempts = 1;
        let first = fixture.enqueue(&original, "old-owner").await;
        let recovered = fixture
            .queue
            .reclaim_stale("new-owner", &mut "0-0".to_owned())
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(recovered.entry_id, first.entry_id);
        assert_eq!(recovered.job.attempts, 1);
        retry_or_dead_letter(
            &fixture.queue,
            &recovered.job,
            &recovered.entry_id,
            "new-owner",
            "Recovered uncertain job",
        )
        .await;

        let next = fixture
            .queue
            .read_next(&mut fixture.conn.clone(), "next-owner")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.job.attempts, 2);
        assert_ne!(next.entry_id, first.entry_id);
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (0, 1));
    }

    #[tokio::test]
    async fn uncertain_recovery_at_the_attempt_limit_dead_letters_instead_of_reexecuting() {
        let fixture = Fixture::new(2).await;
        let mut original = job();
        original.attempts = 1;
        fixture.enqueue(&original, "old-owner").await;
        let recovered = fixture
            .queue
            .reclaim_stale("new-owner", &mut "0-0".to_owned())
            .await
            .unwrap()
            .pop()
            .unwrap();

        retry_or_dead_letter(
            &fixture.queue,
            &recovered.job,
            &recovered.entry_id,
            "new-owner",
            "Recovered uncertain job",
        )
        .await;

        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending, stats.dead_letter), (0, 0, 1));
        let dead_letter: redis::streams::StreamRangeReply = redis::cmd("XRANGE")
            .arg(&*fixture.queue.cfg().dead_letter_key)
            .arg("-")
            .arg("+")
            .query_async(&mut fixture.conn.clone())
            .await
            .unwrap();
        assert_eq!(dead_letter.ids.len(), 1);
        let payload = dead_letter.ids[0].get::<String>("data").unwrap();
        let failed: DownloadJob = serde_json::from_str(&payload).unwrap();
        assert_eq!(failed.attempts, 2);

        let source: redis::streams::StreamRangeReply = redis::cmd("XRANGE")
            .arg(&*fixture.queue.cfg().stream_key)
            .arg("-")
            .arg("+")
            .query_async(&mut fixture.conn.clone())
            .await
            .unwrap();
        assert!(source.ids.is_empty());
    }

    #[tokio::test]
    async fn forced_worker_cancellation_leaves_the_entry_pending_for_recovery() {
        let fixture = Fixture::new(3).await;
        let queued = fixture.enqueue(&job(), "first").await;
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let queue = Arc::new(fixture.queue);
        let worker = tokio::spawn(async move {
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        started_rx.await.unwrap();
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());

        let recovered = queue
            .reclaim_stale("replacement", &mut "0-0".to_owned())
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(recovered.entry_id, queued.entry_id);
        assert!(recovered.recovered);
        let stats = queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (0, 1));
    }

    #[test]
    fn timeout_is_execution_uncertain() {
        assert!(matches!(JobError::Timeout.failure(), JobFailure::Uncertain));
    }
}
