//! Download worker pool draining the Redis queue.
//!
//! Each worker is a distinct stream consumer that blocks for the next [`DownloadJob`], runs the
//! real download interactor inside a fresh request scope (mirroring what the telers integration
//! does per update), then acks. Jobs left pending by a crashed worker are recovered on startup via
//! [`RedisJobQueue::reclaim_stale`]. Concurrency is bounded by the number of workers, which is the
//! cap that smooths the post-restart backlog burst.

use std::{sync::Arc, time::Duration};

use froodi::{async_impl::Container, DefaultScope::Request, ResolveErrorKind, ScopeWithErrorKind};
use redis::aio::ConnectionManager;
use telers::errors::HandlerError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};

use crate::{
    entities::{DownloadJob, JobTarget},
    interactors::{audio, chosen_inline, photo, video, Interactor as _},
    services::{
        messenger::telegram::TelegramMessenger,
        queue::{QueuedJob, RedisJobQueue},
    },
    value_objects::MediaType,
};

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
}

/// Ensures the consumer group exists and spawns `workers` worker tasks. Returns their join handles
/// so the caller can await graceful drain after cancelling `shutdown`.
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
            let consumer = format!("worker-{id}");
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

    // Recover jobs an earlier (crashed) run left pending for this consumer slot.
    match queue.reclaim_stale(&consumer).await {
        Ok(reclaimed) => {
            for queued in reclaimed {
                process(container.clone(), &queue, queued).await;
            }
        }
        Err(err) => error!(%err, "Reclaim stale jobs error"),
    }

    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            res = queue.read_next(&mut read_conn, &consumer) => match res {
                Ok(Some(queued)) => process(container.clone(), &queue, queued).await,
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

#[instrument(skip_all, fields(job_id = %job.job_id, entry_id = entry_id))]
async fn process(container: Container, queue: &RedisJobQueue, QueuedJob { entry_id, job }: QueuedJob) {
    // Best-effort dedup: a job replayed after a crash-before-ack must not be delivered twice.
    match queue.is_done(job.job_id).await {
        Ok(true) => {
            let _ = queue.ack(&entry_id).await;
            return;
        }
        Ok(false) => {}
        Err(err) => warn!(%err, "Dedup check error; processing anyway"),
    }

    info!("Processing download job");
    match run_job(container, &job).await {
        Ok(()) => {
            let _ = queue.mark_done(job.job_id).await;
            if let Err(err) = queue.ack(&entry_id).await {
                error!(%err, "Ack job error");
            }
        }
        Err(err) => {
            error!(%err, "Job failed");
            let mut job = job;
            job.attempts += 1;
            if job.attempts >= queue.cfg().max_attempts {
                warn!(attempts = job.attempts, "Job exceeded max attempts; dead-lettering");
                let _ = queue.dead_letter(&job, &entry_id).await;
            } else if let Err(err) = queue.requeue(&job, &entry_id).await {
                error!(%err, "Requeue job error");
            }
        }
    }
}

async fn run_job(container: Container, job: &DownloadJob) -> Result<(), JobError> {
    let child = container.enter().with_scope(Request).build()?;
    let result = run_in_scope(&child, job).await;
    child.close().await;
    result
}

async fn run_in_scope(child: &Container, job: &DownloadJob) -> Result<(), JobError> {
    match &job.target {
        JobTarget::Command { chat_id, message_id } => {
            let url = job.url.as_ref().ok_or(JobError::MissingUrl)?;
            // Each media type has its own interactor + `Input` type (same fields, distinct types),
            // so these can't collapse into a macro the way the inline arm below does.
            match job.media_type {
                MediaType::Video => {
                    let interactor = child.get::<video::Download<TelegramMessenger>>().await?;
                    interactor
                        .execute(video::DownloadInput {
                            message_id: *message_id,
                            chat_id: *chat_id,
                            params: &job.params,
                            url,
                            chat_cfg: &job.chat_cfg,
                            link_is_visible: job.link_is_visible,
                        })
                        .await?;
                }
                MediaType::Audio => {
                    let interactor = child.get::<audio::Download<TelegramMessenger>>().await?;
                    interactor
                        .execute(audio::DownloadInput {
                            message_id: *message_id,
                            chat_id: *chat_id,
                            params: &job.params,
                            url,
                            chat_cfg: &job.chat_cfg,
                            link_is_visible: job.link_is_visible,
                        })
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
                        })
                        .await?;
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
                        })
                        .await?;
                }};
            }
            match job.media_type {
                MediaType::Video => run!(chosen_inline::DownloadVideo<TelegramMessenger>),
                MediaType::Audio => run!(chosen_inline::DownloadAudio<TelegramMessenger>),
                MediaType::Photo => run!(chosen_inline::DownloadPhoto<TelegramMessenger>),
            }
        }
    }
    Ok(())
}
