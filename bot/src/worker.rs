//! One Redis dispatcher with node-derived backpressure and concurrent job tasks.
//! Known and ambiguous outcomes finish their entry; abandoned deliveries are discarded.

use std::{future::Future, panic::AssertUnwindSafe, sync::Arc, time::Duration};

use downloader_client::outcome::ExecutionOutcome;
use downloader_client::Capacity;
use froodi::{async_impl::Container, DefaultScope::Request, ScopeWithErrorKind};
use futures_util::FutureExt as _;
use redis::aio::ConnectionManager;
use rust_i18n::t;
use telers::{errors::HandlerError, methods::SetMessageReaction, Bot};
use tokio::{
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::{
    config::{DomainsWithReactionsConfig, PROGRESS_REFRESH_INTERVAL_SECS},
    entities::{DownloadJob, JobTarget},
    handlers_utils::progress,
    interactors::{audio, auto, chosen_inline, photo, video, Interactor as _},
    services::{
        messenger::telegram::TelegramMessenger,
        node_router::NodeRouter,
        queue::{QueuedJob, RedisJobQueue},
    },
    value_objects::MediaType,
};

const SHUTDOWN_GRACE: Duration = Duration::from_secs(20);
const PROGRESS_REFRESH_INTERVAL: Duration = Duration::from_secs(PROGRESS_REFRESH_INTERVAL_SECS);

#[derive(Debug, thiserror::Error)]
enum JobError {
    #[error(transparent)]
    Scope(#[from] ScopeWithErrorKind),
    #[error(transparent)]
    Handler(#[from] HandlerError),
    #[error("Command job is missing its URL")]
    MissingUrl,
}

/// Starts one consumer. Its `JoinSet` owns every execution, including during shutdown.
pub async fn spawn_dispatcher(container: Container, shutdown: CancellationToken) -> JoinHandle<()> {
    let queue = container.get::<RedisJobQueue>().await.unwrap();
    let router = container.get::<NodeRouter>().await.unwrap();
    queue.ensure_group().await.unwrap();
    let read_conn = container.get_transient::<ConnectionManager>().await.unwrap();
    let capacity = router.subscribe_capacity();
    tokio::spawn(dispatch(queue.clone(), read_conn, capacity, shutdown, move |queued, consumer| {
        let container = container.clone();
        let queue = queue.clone();
        async move { process(container, &queue, &consumer, queued).await }
    }))
}

async fn dispatch<F, Fut>(
    queue: Arc<RedisJobQueue>,
    mut read_conn: ConnectionManager,
    mut capacity: watch::Receiver<Capacity>,
    shutdown: CancellationToken,
    mut execute: F,
) where
    F: FnMut(QueuedJob, String) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let consumer = format!("bot-instance-{}", Uuid::now_v7());
    let mut credits = capacity.borrow_and_update().dispatch_budget(0);
    let mut tasks = JoinSet::new();
    let mut cursor = "0-0".to_owned();
    let mut next_cleanup = Instant::now();
    info!(%consumer, "Starting download dispatcher");
    loop {
        while let Some(result) = tasks.try_join_next() {
            if let Err(err) = result {
                error!(%err, "Download job task failed; entry remains pending");
            }
        }
        if shutdown.is_cancelled() {
            break;
        }
        if Instant::now() >= next_cleanup {
            if let Err(err) = queue.cleanup_stale(&consumer, &mut cursor).await {
                error!(%err, "Failed to clean abandoned queue entries");
            }
            next_cleanup = Instant::now() + Duration::from_secs(5);
        }
        if capacity.has_changed().unwrap_or(false) {
            // A status generation can be spent only once. Local task count caps
            // pre-dispatch work when successive snapshots have not observed it yet.
            credits = capacity.borrow_and_update().dispatch_budget(tasks.len());
        }
        if credits == 0 {
            tokio::select! {
                () = shutdown.cancelled() => break,
                () = tokio::time::sleep_until(next_cleanup) => {}
                changed = capacity.changed() => {
                    if changed.is_err() { break; }
                    credits = capacity.borrow_and_update().dispatch_budget(tasks.len());
                }
                result = tasks.join_next(), if !tasks.is_empty() => {
                    if let Some(Err(err)) = result { error!(%err, "Download job task failed"); }
                }
            }
            continue;
        }
        let next = tokio::select! {
            () = shutdown.cancelled() => break,
            next = queue.read_next(&mut read_conn, &consumer) => next,
        };
        if shutdown.is_cancelled() {
            break;
        }
        match next {
            Ok(Some(queued)) => {
                credits -= 1;
                tasks.spawn(execute(queued, consumer.clone()));
            }
            Ok(None) => {}
            Err(err) => {
                error!(%err, "Read queued job error");
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
            }
        }
    }
    drain(&mut tasks, SHUTDOWN_GRACE).await;
    info!("Download dispatcher stopped");
}

async fn drain(tasks: &mut JoinSet<()>, grace: Duration) {
    if tokio::time::timeout(grace, async { while tasks.join_next().await.is_some() {} })
        .await
        .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
}

#[instrument(skip_all, fields(job_id = %queued.job.job_id, entry_id = queued.entry_id))]
async fn process(container: Container, queue: &RedisJobQueue, consumer: &str, queued: QueuedJob) {
    let outcome = ExecutionOutcome::default();
    let initial_message = match queued.job.target {
        JobTarget::Command { chat_id, .. } => queued.job.progress_message_id.map(|id| (chat_id, id)),
        JobTarget::Inline { .. } => None,
    };
    let (current_message, message) = watch::channel(initial_message);
    let running = outcome.scope(process_inner(
        container.clone(),
        queue,
        consumer,
        &queued,
        &outcome,
        &current_message,
    ));
    if !run_with_queue_liveness(running, queue, consumer, &queued.entry_id, &outcome, PROGRESS_REFRESH_INTERVAL).await {
        warn!("Job lost queue liveness; stopping execution");
        let notification = tokio::time::timeout(Duration::from_secs(30), async {
            let messenger = container.get::<TelegramMessenger>().await.unwrap();
            let text = t!("download.execution_unknown", locale = queued.job.chat_cfg.locale().as_str());
            match &queued.job.target {
                JobTarget::Command { chat_id, message_id } => {
                    let current = *message.borrow();
                    if let Some((chat_id, message_id)) = current {
                        progress::is_error_in_progress(messenger.as_ref(), chat_id, message_id, &text, None).await
                    } else {
                        progress::new(messenger.as_ref(), &text, *chat_id, Some(*message_id), None)
                            .await
                            .map(|_| ())
                    }
                }
                JobTarget::Inline { inline_message_id, .. } => {
                    progress::is_error_in_chosen_inline(messenger.as_ref(), inline_message_id, &text, None).await
                }
            }
        })
        .await;
        match notification {
            Ok(Ok(())) => {}
            Ok(Err(err)) => error!(%err, "Send queue liveness error failed"),
            Err(err) => error!(%err, "Send queue liveness error timed out"),
        }
        finish(queue, consumer, &queued, Ok(Ok(())), &outcome).await;
    }
}

async fn run_with_queue_liveness(
    running: impl Future<Output = ()>,
    queue: &RedisJobQueue,
    consumer: &str,
    entry_id: &str,
    outcome: &ExecutionOutcome,
    refresh_interval: Duration,
) -> bool {
    tokio::pin!(running);
    let mut progress = outcome.subscribe_progress();
    let mut refreshed_at = Instant::now();
    loop {
        tokio::select! {
            biased;
            () = &mut running => return true,
            changed = progress.changed() => {
                if changed.is_err() { return false; }
                if refreshed_at.elapsed() >= refresh_interval {
                    match tokio::time::timeout(Duration::from_secs(5), queue.refresh_liveness(entry_id, consumer)).await {
                        Ok(Ok(true)) => refreshed_at = Instant::now(),
                        _ => return false,
                    }
                }
            }
        }
    }
}

async fn process_inner(
    container: Container,
    queue: &RedisJobQueue,
    consumer: &str,
    queued: &QueuedJob,
    outcome: &ExecutionOutcome,
    current_message: &watch::Sender<Option<(i64, i64)>>,
) {
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
    info!("Processing download job");
    let result = AssertUnwindSafe(run_job(container, &queued.job, current_message))
        .catch_unwind()
        .await;
    finish(queue, consumer, queued, result, outcome).await;
}

async fn finish(
    queue: &RedisJobQueue,
    consumer: &str,
    queued: &QueuedJob,
    result: std::thread::Result<Result<(), JobError>>,
    outcome: &ExecutionOutcome,
) {
    if outcome.is_uncertain() {
        warn!("Remote execution outcome is uncertain; finishing without automatic re-execution");
    }
    if result.is_err() {
        error!("Download job panicked; finishing without automatic re-execution");
    }
    if let Ok(Err(err)) = result {
        warn!(%err, "Job reached a known failure; finishing queue delivery");
    }
    if let Err(err) = queue.mark_done(queued.job.job_id).await {
        error!(%err, "Mark job done error");
        return;
    }
    match queue.ack(&queued.entry_id, consumer).await {
        Ok(true) => info!("Finished download queue delivery"),
        Ok(false) => warn!("Job is no longer owned by this consumer"),
        Err(err) => error!(%err, "Ack job error"),
    }
}

async fn run_job(container: Container, job: &DownloadJob, current_message: &watch::Sender<Option<(i64, i64)>>) -> Result<(), JobError> {
    let child = container.enter().with_scope(Request).build()?;
    let result = Box::pin(run_in_scope(&child, job, current_message)).await;
    // Clear the acknowledgment reaction once processing is done, on success or failure alike (the
    // handler only enqueued). No-op for inline jobs and links the reaction middleware ignored.
    if let JobTarget::Command { chat_id, message_id } = &job.target {
        let _ = tokio::time::timeout(Duration::from_secs(5), clear_reaction(&child, job, *chat_id, *message_id)).await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), child.close()).await;
    result
}

async fn run_in_scope(child: &Container, job: &DownloadJob, current_message: &watch::Sender<Option<(i64, i64)>>) -> Result<(), JobError> {
    match &job.target {
        JobTarget::Command { chat_id, message_id } => {
            let url = job.url.as_ref().ok_or(JobError::MissingUrl)?;
            if job.auto {
                // Bare link with no committed type: classify (video -> audio -> photo) and download.
                if job.quiet {
                    let interactor = child.get::<auto::AutoQuiet<TelegramMessenger>>().await.unwrap();
                    Box::pin(interactor.execute(auto::AutoQuietInput {
                        message_id: *message_id,
                        chat_id: *chat_id,
                        params: &job.params,
                        url,
                        link_is_visible: job.link_is_visible,
                    }))
                    .await?;
                } else {
                    let interactor = child.get::<auto::Auto<TelegramMessenger>>().await.unwrap();
                    Box::pin(interactor.execute(auto::AutoInput {
                        current_message,
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
                        let interactor = child.get::<video::Download<TelegramMessenger>>().await.unwrap();
                        Box::pin(interactor.execute(video::DownloadInput {
                            current_message,
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
                        let interactor = child.get::<audio::Download<TelegramMessenger>>().await.unwrap();
                        Box::pin(interactor.execute(audio::DownloadInput {
                            current_message,
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
                        let interactor = child.get::<photo::Download<TelegramMessenger>>().await.unwrap();
                        interactor
                            .execute(photo::DownloadInput {
                                current_message,
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
                    let interactor = child.get::<$download>().await.unwrap();
                    interactor
                        .execute(chosen_inline::DownloadInput {
                            guest: job.guest,
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
    let domains_with_reactions = child.get::<DomainsWithReactionsConfig>().await.unwrap();
    if !domains_with_reactions
        .domains
        .contains(&domain.trim_start_matches("www.").to_owned())
    {
        return;
    }
    let bot = child.get::<Bot>().await.unwrap();
    if let Err(err) = bot.send(SetMessageReaction::new(chat_id, message_id)).await {
        error!(%err, "Unset reaction error");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::queue::test_support::Fixture;

    #[tokio::test]
    async fn progressing_playlist_outlives_cleanup_but_abandoned_work_is_discarded() {
        let fixture = Fixture::with_config(700).await;
        let queued = fixture.enqueue().await;
        let outcome = ExecutionOutcome::default();
        let playlist = async {
            for _ in 0..12 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                outcome.record_progress();
                assert_eq!(fixture.queue.cleanup_stale("cleanup", &mut "0-0".into()).await.unwrap(), 0);
            }
        };
        assert!(
            run_with_queue_liveness(
                playlist,
                &fixture.queue,
                "first",
                &queued.entry_id,
                &outcome,
                Duration::from_millis(100),
            )
            .await
        );
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if fixture.queue.cleanup_stale("cleanup", &mut "0-0".into()).await.unwrap() > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        assert!(!fixture.queue.ack(&queued.entry_id, "first").await.unwrap());
        assert_eq!(fixture.queue.stats().await.unwrap().pending, 0);
    }

    #[tokio::test]
    async fn intentional_wait_after_uncertainty_protects_delivery_until_waiter_stops() {
        let fixture = Fixture::with_config(700).await;
        let queued = fixture.enqueue().await;
        let outcome = ExecutionOutcome::default();
        outcome.mark_uncertain();
        let wait = async {
            for _ in 0..12 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                outcome.record_progress();
                assert_eq!(fixture.queue.cleanup_stale("cleanup", &mut "0-0".into()).await.unwrap(), 0);
            }
        };
        assert!(
            run_with_queue_liveness(
                wait,
                &fixture.queue,
                "first",
                &queued.entry_id,
                &outcome,
                Duration::from_millis(100),
            )
            .await
        );
        assert_eq!(fixture.queue.stats().await.unwrap().pending, 1);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if fixture.queue.cleanup_stale("cleanup", &mut "0-0".into()).await.unwrap() == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(fixture.queue.stats().await.unwrap().pending, 0);
    }

    #[tokio::test]
    async fn stale_progress_cannot_restore_a_cleaned_entry() {
        let fixture = Fixture::new().await;
        let queued = fixture.enqueue().await;
        fixture.queue.cleanup_stale("cleanup", &mut "0-0".into()).await.unwrap();
        let outcome = ExecutionOutcome::default();
        let stale_work = async {
            outcome.record_progress();
            std::future::pending::<()>().await;
        };
        assert!(!run_with_queue_liveness(stale_work, &fixture.queue, "first", &queued.entry_id, &outcome, Duration::ZERO,).await);
        assert!(!fixture.queue.refresh_liveness(&queued.entry_id, "first").await.unwrap());
        assert!(!fixture.queue.refresh_liveness(&queued.entry_id, "cleanup").await.unwrap());
        assert!(!fixture.queue.ack(&queued.entry_id, "first").await.unwrap());
        assert!(!fixture.queue.ack(&queued.entry_id, "cleanup").await.unwrap());
    }

    #[tokio::test]
    async fn lack_of_progress_does_not_abort_processing() {
        let fixture = Fixture::new().await;
        let queued = fixture.enqueue().await;
        let outcome = ExecutionOutcome::default();
        assert!(tokio::time::timeout(
            Duration::from_millis(30),
            run_with_queue_liveness(
                std::future::pending(),
                &fixture.queue,
                "first",
                &queued.entry_id,
                &outcome,
                Duration::ZERO
            )
        )
        .await
        .is_err());
        assert_eq!(fixture.queue.stats().await.unwrap().pending, 1);
    }
    use tokio::sync::{mpsc, Semaphore};

    #[tokio::test]
    async fn known_success_and_failure_finish_without_replacement() {
        let fixture = Fixture::new().await;
        for result in [Ok(()), Err(JobError::MissingUrl)] {
            let queued = fixture.enqueue().await;
            finish(&fixture.queue, "first", &queued, Ok(result), &ExecutionOutcome::default()).await;
            let stats = fixture.queue.stats().await.unwrap();
            assert_eq!((stats.waiting, stats.pending), (0, 0));
            assert!(fixture.queue.is_done(queued.job.job_id).await.unwrap());
            assert!(fixture.queue.read_next(&mut fixture.connection(), "next").await.unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn ambiguous_execution_and_panic_finish_without_replacement() {
        let fixture = Fixture::new().await;
        let queued = fixture.enqueue().await;
        let outcome = ExecutionOutcome::default();
        outcome.mark_uncertain();
        finish(&fixture.queue, "first", &queued, Ok(Ok(())), &outcome).await;
        assert!(fixture.queue.is_done(queued.job.job_id).await.unwrap());
        let panicked = fixture.enqueue().await;
        finish(&fixture.queue, "first", &panicked, Err(Box::new(())), &ExecutionOutcome::default()).await;
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (0, 0));
    }

    #[tokio::test]
    async fn abandoned_entry_is_cleaned_without_running_a_job() {
        let fixture = Fixture::new().await;
        let queued = fixture.enqueue().await;
        assert_eq!(fixture.queue.cleanup_stale("cleanup", &mut "0-0".into()).await.unwrap(), 1);
        assert!(!fixture.queue.ack(&queued.entry_id, "first").await.unwrap());
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (0, 0));
    }

    #[tokio::test]
    async fn dispatcher_exploits_capacity_and_does_not_spend_a_stale_sample_twice() {
        let fixture = Fixture::with_config(30_000).await;
        let seed = fixture.enqueue().await;
        fixture.queue.ack(&seed.entry_id, "first").await.unwrap();
        for _ in 0..8 {
            fixture.queue.enqueue(&seed.job).await.unwrap();
        }
        let (status, capacity) = watch::channel(Capacity { available: 5, total: 5 });
        let (started, mut starts) = mpsc::unbounded_channel();
        let gate = Arc::new(Semaphore::new(0));
        let shutdown = CancellationToken::new();
        let queue = fixture.queue.clone();
        let task_queue = queue.clone();
        let task_gate = gate.clone();
        let dispatcher = tokio::spawn(dispatch(
            queue,
            fixture.connection(),
            capacity,
            shutdown.clone(),
            move |queued, consumer| {
                let queue = task_queue.clone();
                let gate = task_gate.clone();
                let started = started.clone();
                async move {
                    started.send(queued.entry_id.clone()).unwrap();
                    let permit = gate.acquire().await.unwrap();
                    permit.forget();
                    queue.ack(&queued.entry_id, &consumer).await.unwrap();
                }
            },
        ));
        for _ in 0..5 {
            tokio::time::timeout(Duration::from_secs(3), starts.recv()).await.unwrap().unwrap();
        }
        assert_eq!(fixture.queue.stats().await.unwrap().waiting, 3);
        status.send_replace(Capacity { available: 5, total: 5 });
        assert!(tokio::time::timeout(Duration::from_millis(100), starts.recv()).await.is_err());
        status.send_replace(Capacity { available: 3, total: 8 });
        for _ in 0..3 {
            tokio::time::timeout(Duration::from_secs(3), starts.recv()).await.unwrap().unwrap();
        }
        assert_eq!(fixture.queue.stats().await.unwrap().pending, 8);
        gate.add_permits(8);
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(3), dispatcher).await.unwrap().unwrap();
        assert_eq!(fixture.queue.stats().await.unwrap().pending, 0);
    }

    #[tokio::test]
    async fn full_cluster_keeps_work_undelivered_and_idle_shutdown_is_prompt() {
        let fixture = Fixture::new().await;
        let seed = fixture.enqueue().await;
        fixture.queue.ack(&seed.entry_id, "first").await.unwrap();
        fixture.queue.enqueue(&seed.job).await.unwrap();
        let (_status, capacity) = watch::channel(Capacity { available: 0, total: 5 });
        let shutdown = CancellationToken::new();
        let dispatcher = tokio::spawn(dispatch(
            fixture.queue.clone(),
            fixture.connection(),
            capacity,
            shutdown.clone(),
            |_, _| async {
                panic!("Full capacity snapshot must not claim a job");
            },
        ));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let stats = fixture.queue.stats().await.unwrap();
        assert_eq!((stats.waiting, stats.pending), (1, 0));
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(1), dispatcher).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn dispatcher_discards_abandoned_delivery_without_executing_it() {
        let fixture = Fixture::with_config(50).await;
        let queued = fixture.enqueue().await;
        let (_status, capacity) = watch::channel(Capacity { available: 0, total: 0 });
        let shutdown = CancellationToken::new();
        let dispatcher = tokio::spawn(dispatch(
            fixture.queue.clone(),
            fixture.connection(),
            capacity,
            shutdown.clone(),
            |_, _| async { panic!("Abandoned work must not execute") },
        ));
        tokio::time::timeout(Duration::from_secs(7), async {
            loop {
                if fixture.queue.stats().await.unwrap().pending == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        assert!(!fixture.queue.ack(&queued.entry_id, "first").await.unwrap());
        shutdown.cancel();
        dispatcher.await.unwrap();
    }

    #[tokio::test]
    async fn forced_shutdown_aborts_owned_tasks_and_leaves_cleanup_entries() {
        let fixture = Fixture::new().await;
        let queued = fixture.enqueue().await;
        let mut tasks = JoinSet::new();
        tasks.spawn(std::future::pending::<()>());
        drain(&mut tasks, Duration::from_millis(10)).await;
        assert!(tasks.is_empty());
        assert_eq!(fixture.queue.stats().await.unwrap().pending, 1);
        assert_eq!(fixture.queue.cleanup_stale("cleanup", &mut "0-0".into()).await.unwrap(), 1);
        assert!(!fixture.queue.ack(&queued.entry_id, "first").await.unwrap());
    }
}
