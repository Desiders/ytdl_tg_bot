# ytdl_tg_bot Workspace Guide

## Purpose

This workspace contains a Telegram bot, a shared downloader-client crate, a gRPC downloader service, a separate cookie-assignment controller, and the database migration crate.

- `bot`: accepts Telegram updates, enqueues download jobs into a durable Valkey/Redis stream, runs a worker pool that drains that queue, caches Telegram `file_id`s in PostgreSQL, routes downloads to downloader nodes, and uploads media to Telegram
- `downloader_client`: shared downloader-node discovery, mTLS client setup, routing, downloader failover, and RPC adapters for every downloader-node service
- `downloader`: runs `yt-dlp` and `gallery-dl`, resolves DRM music links through `spotdl`, resolves Instagram/Facebook links through snapsave, recognizes songs through `SongRec`, fetches and embeds thumbnails, and streams results back over gRPC
- `cookie_assignment`: discovers downloader nodes and pushes cookie files to them over gRPC
- `migration`: SeaORM migrations for the bot database; also used by bot tests to build a schema in a throwaway PostgreSQL container
- `proto`: shared protobuf definitions used by the runtime services

This file describes the current architecture and the rules future changes should follow. It is not an implementation plan.

## Workspace Layout

- root workspace members: `bot`, `cookie_assignment`, `downloader`, `downloader_client`, `migration`, `proto`
- helm charts:
  - `charts/infra`
  - `charts/bot`
  - `charts/cookie-assignment`
  - `charts/downloader`
- config templates (committed) and local configs (gitignored):
  - `configs/config.example.toml` -> `configs/config.toml`
  - `configs/cookie_assignment.example.toml` -> `configs/cookie_assignment.toml`
  - `configs/downloader.example.toml` -> `configs/downloader.toml`
- config path env vars: `CONFIG_PATH` (bot), `DOWNLOADER_CONFIG_PATH`, `COOKIE_ASSIGNMENT_CONFIG_PATH`
- `DOWNLOADER_SERVICE_DNS` (`host:port`) is required by the bot and the cookie-assignment controller; the charts set it to `downloader.<namespace>.svc.cluster.local:50051`
- Dockerfiles live in `deployment/` (`Dockerfile.<component>` for release, `Dockerfile.<component>.dev` for dev images built by `scripts/build-*-dev-image.sh`)
- bot locales live in `bot/locales/{en,ru,uk}.toml`; every new text key must be added to all three
- `justfile` is the entry point for lint, fmt, images, Helm, and Kubernetes helpers

## Kubernetes Deployment & Infrastructure

The project is deployed to Kubernetes with separate Helm charts for:

- shared infrastructure (`charts/infra`): the internal CA `ClusterIssuer`
- bot runtime (`charts/bot`): bot Deployment, PostgreSQL (CloudNativePG), Valkey (`ValkeyCluster`), internal RustFS backup storage, PostgreSQL scheduled backup, local Telegram Bot API server with an nginx file server, yt-toolkit, and the migration Job template
- downloader nodes (`charts/downloader`): headless `downloader` Service, worker Deployment, and the shared `yt-pot-provider`
- cookie assignment (`charts/cookie-assignment`)

Cluster prerequisites: `cert-manager`, CloudNativePG `1.26+` (prefer `1.29+`), the Barman Cloud CNPG-I Plugin, and the Valkey operator. Install the infra chart before the app charts so `ca-issuer` exists before certificate resources are reconciled.

Versions to keep in sync when releasing a component: the crate `version` in `Cargo.toml`, `appVersion` in the chart `Chart.yaml`, and the image tag in the chart `values.yaml`.

### PostgreSQL Backup

The bot chart uses the current CloudNativePG plugin-based backup path.

- `charts/bot` creates a `barmancloud.cnpg.io/v1` `ObjectStore`.
- The `postgresql.cnpg.io/v1` `Cluster` references that object store through `spec.plugins`.
- `ScheduledBackup` uses `method: plugin` with `barman-cloud.cloudnative-pg.io`.
- Do not reintroduce `Cluster.spec.backup.barmanObjectStore`; that is the deprecated in-tree Barman Cloud path.
- CNPG backup schedules use six-field cron syntax with seconds first, for example `0 0 3 * * *`.
- The default chart includes single-node RustFS and bootstraps the `backups` bucket through a Helm hook Job.
- If RustFS is disabled and an external S3-compatible store is used, the bucket must exist before backups run.

### Valkey Queue

- `charts/bot` creates a `valkey.io/v1alpha1` `ValkeyCluster` named `valkey` with AOF persistence and `maxmemory-policy: noeviction`.
- The `admin` user password comes from the `valkey` Secret; bot config `[redis]` must use user `admin` and the same password.
- The queue is the only Valkey consumer. Do not put caches that may be evicted into the same instance without changing the eviction policy.

### Local Telegram Bot API

- The bot talks to a self-hosted `telegram-bot-api` in `--local` mode. In local mode `getFile` returns an absolute path and the server does not serve files over HTTP.
- The chart runs an nginx file server sidecar exposing the bot API work dir, and the bot downloads files from `[telegram_bot_api].file_server_url` by stripping `[telegram_bot_api].work_dir` from the path, then deletes the file over WebDAV `DELETE`.
- Omit `file_server_url` and `work_dir` when pointing the bot at the cloud API.

### Service Discovery (Dynamic Nodes)

The bot and the cookie-assignment controller do **not** use hardcoded node IPs or static node lists.

- Downloader nodes are discovered dynamically via Kubernetes Headless Service DNS.
- Clients resolve the `DOWNLOADER_SERVICE_DNS` authority, `downloader.<namespace>.svc.cluster.local:50051` in the charts.
- The lookup returns active pod IPs.
- The bot passes resolved endpoints into `NodeRouter`.
- The cookie-assignment controller uses the same DNS name for assignment cycles.

### Mutual TLS (mTLS) Constraints

All internal service-to-node communication uses strict mTLS.

- Certificates are issued by `cert-manager` with the shared internal CA.
- The infra chart owns the shared `ca-issuer` resources.
- Certificates and keys are mounted into containers at hardcoded paths. If you change the paths, update code and Helm together.

**Bot TLS paths:**

- CA: `/app/tls/ca.crt`
- Cert: `/app/tls/bot.crt`
- Key: `/app/tls/bot.key`

**Cookie-assignment TLS paths:**

- CA: `/app/tls/ca.crt`
- Cert: `/app/tls/cookie-assignment.crt`
- Key: `/app/tls/cookie-assignment.key`

**Downloader TLS paths:**

- CA: `/app/tls/ca.crt`
- Cert: `/app/tls/node.crt`
- Key: `/app/tls/node.key`

**Critical TLS Rule (SNI / ServerName):**

Both the bot and the cookie-assignment controller connect to nodes by IP address, but the downloader certificate SAN contains the DNS name. The gRPC TLS client **MUST** set `server_name` / SNI to `downloader.<namespace>.svc.cluster.local`. `downloader_client` takes that name from the host part of `DOWNLOADER_SERVICE_DNS`.

If this rule is broken, the TLS handshake fails with certificate verification errors.

Client channels also use HTTP/2 keepalive (30s interval, 20s timeout, while idle) so a half-open node connection surfaces as a stream error instead of a hang. Keep that when touching `downloader_client/src/client.rs`.

## Current Runtime Roles

### Bot

- accepts Telegram updates through long polling
- cleans incoming URLs with `clear-urls` embedded rules plus `[tracking_params]` (`CleanUrlMiddleware`)
- filters links: `[blacklisted].domains`, per-chat excluded domains, `yv2t_bot=false`-style skip params, and messages sent via the bot itself
- sets an acknowledgment reaction for `[domains_with_reactions]` on receipt; the worker clears it when the job finishes
- enqueues every download (command, bare link, chosen inline result) as a `DownloadJob` into the Valkey stream
- runs `[redis.queue].workers` worker tasks that pull jobs and run the download interactors
- fetches media metadata through downloader nodes, with a PostgreSQL `file_id` cache in front (`downloaded_media`)
- resolves Spotify links to DRM-free sources through the node `MusicResolver` before metadata lookup
- uses yt-toolkit for YouTube inline info and inline text search
- selects nodes through `NodeRouter`
- downloads media streams from downloader nodes and forwards media plus thumbnail streams to Telegram
- refreshes node status every 5 seconds and node cookie capabilities every `[download].capabilities_refresh_interval` seconds (0 disables)
- recognizes songs for `/shazam` by downloading the Telegram file and calling the node `SongRecognizer`, then enqueues the found track as an audio download
- serves `en`, `ru`, and `uk` locales; `/lang` switches a chat's locale

### Bot Queue Rules

- Handlers never download. They inject `EnqueueCommandDownload` or `EnqueueInlineDownload` and return.
- `bot/src/services/queue.rs` owns the stream protocol: `XADD` on enqueue, `XREADGROUP` per worker consumer, `XACK` + `XDEL` on ack, `XAUTOCLAIM` to recover jobs from crashed workers, a per-`job_id` done marker with TTL for best-effort dedup, and a dead-letter stream after `max_attempts`.
- `bot/src/worker.rs` owns job execution: each job runs in a fresh request-scoped DI container under `[timeouts].job` seconds, dispatches on `JobTarget` (`Command` or `Inline`) and `auto` / `media_type`, clears the acknowledgment reaction, then acks, requeues, or dead-letters.
- `DownloadJob` is serialized as JSON. New fields must be `#[serde(default)]` so jobs written by an older bot version still deserialize after a rollout.
- Shutdown cancels the workers and waits for in-flight jobs; anything still pending stays in the stream's pending list and is reclaimed on the next start.
- `/stats` reports queue waiting, in-progress, and dead-letter counts alongside node and cache stats.

### Bot Messenger Boundary

- Outbound messenger operations should go through `MessengerPort` (`bot/src/services/messenger.rs`).
- `TelegramMessenger` (`bot/src/services/messenger/telegram.rs`) is the adapter that owns Telegram API calls, retries, parse modes, inline answer wiring, and Telegram media send/edit method construction.
- Bot handlers, top-level bot interactors, and `send_media` interactors should depend on the messenger port layer, not construct Telegram methods directly.
- Keep Telegram SDK/API types isolated to the Telegram adapter. Utility string helpers such as HTML escaping may still live elsewhere, but Telegram request construction should have one source of truth.
- Current known exceptions that use `telers::Bot` directly: `ReactionMiddleware`, `worker::clear_reaction`, startup `SetMyCommands`, and `TelegramFileDownloader` (`getFile`). Do not add new ones.

### Bot Handler / Interactor Boundary

- Handlers are Telegram inbound adapters.
- A handler should:
  - extract Telegram input
  - inject one top-level interactor
  - call it
  - return `EventReturn::Finish`
- Do not let handlers orchestrate business flow across multiple top-level interactors or services again.
- Handler-facing orchestration lives in `bot/src/interactors/`:
  - `start`, `stats`, `config`, `lang`, `inline_query`, `shazam`
  - `enqueue_download` (the only download interactors handlers call)
  - `video`, `audio`, `photo`, `auto`, `chosen_inline` (run by the worker, not by handlers)
- Lower-level reusable building blocks live under `bot/src/services/`, not in the interactor namespace:
  - `chat`
  - `download`
  - `downloaded_media`
  - `file_download`
  - `get_media`
  - `messenger`
  - `node_router`
  - `queue`
  - `send_media`
  - `yt_toolkit`
- Top-level interactors may call these services.
- Services must not call top-level interactors.
- `auto` classifies a bare link video -> audio -> photo and reuses the type-specific interactors through `prefetched` metadata; `AutoQuiet` is the group-chat variant with no progress messages.

### Bot DI Style

- Keep the current generic DI style around `Messenger`.
- The composition root (`bot/src/di_container.rs`) has separate registries: `cfg_registry`, `tg_messenger_registry`, `node_router_registry`, `interactors_registry::<Messenger>`, `database_registry`, and `queue_registry`, combined by `init`.
- The composition root builds `TelegramMessenger`, but top-level interactors should be wired generically over `Messenger` rather than directly against the concrete adapter type.
- If you add a new top-level interactor, register it in `interactors_registry<Messenger>(...)` and keep the same generic pattern.
- Anything the worker needs must be resolvable from a request scope of the container, since jobs run outside the telers update pipeline.

### Cookie-assignment Controller

- reads cookie files mounted into its own pod
- discovers downloader nodes through the headless downloader service DNS using `downloader_client`
- checks node availability with `NodeCapabilities.GetStatus` using the cookie-manager token
- lists node cookies with `NodeCookieManager.ListNodeCookies` and treats successful status plus cookie-list responses as worker availability for new assignments during the cycle
- removes stale unassigned cookies from nodes
- pushes free cookies to eligible nodes with `NodeCookieManager.PushCookie`
- keeps assignments in memory only

### Downloader Auth Boundary

Downloader nodes use separate bearer tokens for separate RPC surfaces.

- `Downloader`, `MusicResolver`, and `SongRecognizer` accept any token listed in downloader config `[auth].node_tokens`.
- `NodeCapabilities` accepts any token listed in `[auth].node_tokens` or the cookie-manager token.
- `NodeCookieManager` uses the cookie-manager token only.
- `[auth].node_tokens` must not be empty; the node refuses to start otherwise.
- Each bot config must only contain one normal node token, and that token must be present in downloader `[auth].node_tokens`.
- The cookie-assignment config must only contain the cookie-manager token.
- Do not give the bot the cookie-manager token.

### Adding Another Bot

New messenger bots should be separate applications, not modes inside the Telegram bot.

- Add a new crate such as `bot_discord` when another messenger is needed.
- The new bot should depend on `downloader_client` directly for downloader DNS discovery, mTLS setup, node routing, failover, media info, download streams, DRM resolution, and song recognition.
- Do not depend on the existing `bot` crate from another bot.
- Add an independent chart for the new bot. Copying `charts/bot` as a starting point is acceptable, but remove resources that are not needed by the new bot.
- The current `charts/bot` PostgreSQL, RustFS, Valkey, migrations, Telegram Bot API, yt-toolkit, and upload cache resources are Telegram-bot runtime choices, not mandatory shared infrastructure for every bot.
- Another bot may be simple and have no database, queue, or upload cache.
- If another bot needs a cache, design it around that messenger's remote media identifiers. Do not reuse Telegram `file_id` semantics as shared state.
- Each bot must have its own config Secret and client TLS certificate.
- Each bot config should contain exactly one normal downloader token, and downloader config `[auth].node_tokens` must include that token.
- Never give a bot the cookie-manager token.
- Keep downloader nodes and cookie assignment shared only through `downloader_client`, gRPC, mTLS, and Kubernetes service discovery.

### Downloader Node

- exposes `Downloader`, `NodeCapabilities`, `NodeCookieManager`, `MusicResolver`, and `SongRecognizer` gRPC services on `[server].address`
- limits concurrent downloads with a semaphore of `[server].max_concurrent`; a full node answers `RESOURCE_EXHAUSTED` with `Node is at capacity`
- maps retryable `yt-dlp` failures (login required, geo restriction, anti-bot) to `ABORTED` so clients can retry on another node
- stores assigned cookies only in `/tmp/cookies`, one file per domain (`/tmp/cookies/<domain>.txt`, `www.` stripped)
- clears `/tmp/cookies` on startup
- reports supported domains from currently assigned in-memory cookie state
- never pulls cookies on its own
- media sources by type:
  - video and audio: `yt-dlp`, with `[yt_dlp].extractor_args` and the `yt-pot-provider` sidecar for YouTube; do not add `player_skip=configs`, it breaks adaptive YouTube formats
  - photo: `gallery-dl`
  - Instagram/Facebook without a cookie: snapsave (`[snapsave].enabled`), which yields direct CDN URLs downloaded through `ffmpeg` remux (video), audio extraction (audio), or direct fetch (photo); when snapsave is enabled but unreachable those links fail fast with `NOT_FOUND`; only a disabled snapsave falls back to the domain-replace rule
  - Spotify: `spotdl` (`[spotdl].enabled`) resolves a track, album, or playlist to YouTube URLs; the bot then downloads those
  - `/shazam`: `songrec` (`[songrec].enabled`) with `[songrec].max_audio_size` as the gRPC decoding limit
- applies `[[replace_domains.<video|audio|photo>]]` regex rules only when the node has no cookie for the domain
- applies the first matching `[[user_agents]]` rule (subdomains match) to `yt-dlp` and direct fetches
- formats: video prefers `bv+ba` with combined fallback, audio prefers `ba` with `wa` and `b*` fallbacks; non-fragmentable single formats are piped from `yt-dlp` stdout straight into the stream without an intermediate file
- honors `max_file_size` from the request, capped by the node's own `[yt_dlp].max_file_size`

## Current Download Flow

The bot does not run `yt-dlp` directly.

1. A handler validates the update and calls an `enqueue_download` interactor, which writes a `DownloadJob` to the Valkey stream.
2. A worker pulls the job, opens a request scope, and runs the matching interactor (`video`, `audio`, `photo`, `auto`, or `chosen_inline`).
3. `get_media` checks the PostgreSQL `file_id` cache; on a hit the media is sent by `file_id` and no node is used.
4. For Spotify links the interactor resolves DRM-free URLs through `MusicResolver` first.
5. The bot resolves active downloader node IPs via Kubernetes DNS.
6. The interactor asks `NodeRouter` for a downloader node, preferring one with a cookie for the domain.
7. The bot connects to the chosen node over gRPC using mTLS and the downloader DNS name as SNI.
8. The downloader fetches metadata (`GetMediaInfo`) or downloads media (`DownloadMedia`).
9. The downloader streams progress, then thumbnail and media bytes back to the bot.
10. The bot forwards media and thumbnail streams to Telegram, stores the returned `file_id` in the cache, and clears the acknowledgment reaction.

## Current Stream Contract

Proto file: [proto/proto/downloader.proto](proto/proto/downloader.proto)

`DownloadMeta` currently includes:

- `ext`
- `width`
- `height`
- `duration`
- `has_thumbnail`

`DownloadChunk` currently supports:

- `meta`
- `progress`
- `data`
- `thumbnail_data`

`DownloadMedia` stream order is currently:

1. zero or more `progress`
2. one `meta`
3. zero or more `thumbnail_data` (only when `has_thumbnail` is true)
4. one or more `data`

`meta` is deliberately sent late: after the download succeeded and the output file was validated, or, for piped streaming, at the first media byte. The client (`downloader_client::download_media`) forwards pre-`meta` progress through a callback and returns a `DownloadSession` at `meta`, so a download failure surfaces as an error the bot can fail over or retry with another format instead of corrupting an upload already in progress. Do not emit `meta` early.

`GetMediaInfo` responses may be large; the client decoding limit is 30 MiB. `RecognizeSong` requests are capped at 25 MiB on the client and `[songrec].max_audio_size` on the node.

Do not change any of this casually. If you change it, update bot, `downloader_client`, and downloader together.

## Thumbnail Rules

This is a current invariant.

- The downloader may embed the thumbnail into the final media file.
- The downloader also streams the thumbnail separately to the bot.
- The bot must pass the thumbnail to Telegram as a stream.
- Do not save the thumbnail to a temp file just to upload it.
- `MediaForUpload` uses `thumb_stream`, not `thumb_path`.

## Node Routing

Important behavior:

- node input comes from DNS, not static config
- prefer nodes with cookies for the target domain, then fall back to any node
- skip nodes that are unavailable or already at capacity
- among candidates pick the lowest projected utilization `(active + 1) / max_concurrent`; ties break by fewer active downloads, then larger capacity, then address
- a slot is reserved locally for the duration of each attempt so concurrent picks do not all land on the same node
- retry other nodes on `RESOURCE_EXHAUSTED` and `UNAVAILABLE`
- treat `UNAUTHENTICATED` as a configuration error and surface it as node unavailability to users
- treat retryable `ABORTED` downloader responses as node-context failures and retry other nodes first; if every node fails that way, report the source-site rejection rather than "nodes busy"

Keep download node selection, downloader-node failover, and downloader RPC adapter logic centralized in:

- [`downloader_client/src/router.rs`](downloader_client/src/router.rs)
- [`downloader_client/src/selection.rs`](downloader_client/src/selection.rs)
- [`downloader_client/src/handle.rs`](downloader_client/src/handle.rs)
- [`downloader_client/src/retry.rs`](downloader_client/src/retry.rs)
- [`downloader_client/src/media_info.rs`](downloader_client/src/media_info.rs)
- [`downloader_client/src/download.rs`](downloader_client/src/download.rs)
- [`downloader_client/src/resolve.rs`](downloader_client/src/resolve.rs)
- [`downloader_client/src/recognize.rs`](downloader_client/src/recognize.rs)
- [`downloader_client/src/cookie_assignment.rs`](downloader_client/src/cookie_assignment.rs)

The bot re-exports that crate through `bot/src/services/node_router.rs`, and the cookie-assignment controller keeps its own assignment policy, but DNS discovery, mTLS channel setup, auth request construction, and downloader RPC adapters must stay in `downloader_client`.

## Cookie Lifecycle

The bot no longer owns cookie distribution.

- Cookie files are mounted only into the cookie-assignment deployment.
- Source cookie layout in the repo is `cookies/<domain>/<cookie-id>.txt`; `*.txt` is gitignored.
- `scripts/sync-cookies-secret.sh` (via `just k8s-sync-cookie-assignment-cookies`) flattens entry names to `<domain>__<cookie-id>.txt`; `__` is reserved and nested directories are rejected.
- The cookie-assignment runtime reads flattened `*.txt` files from `/app/cookies` and decodes them back into domain plus file name.
- `cookie_id` is tracked as `<domain>/<cookie-id>.txt` so file names may repeat across domains without colliding.
- Cookie assignments are in-memory only inside the cookie-assignment controller.
- At most one cookie per domain is assigned to a node; free cookies go to the eligible worker with the fewest cookies, then fewest domains, then lowest address.
- The controller assignment cycle currently:
  1. loads source cookies from `/app/cookies`
  2. resolves downloader nodes from DNS
  3. checks each node through `NodeCapabilities.GetStatus` using the cookie-manager token
  4. loads each node's current cookie IDs through `NodeCookieManager.ListNodeCookies`
  5. revokes cookies whose source file disappeared
  6. releases stale in-memory assignments and removes unassigned cookies from nodes
  7. assigns free cookie files to eligible nodes and pushes them via `NodeCookieManager.PushCookie`
- Downloader nodes remain passive:
  - they never pull cookies
  - they clear `/tmp/cookies/` at startup
  - they keep cookies only for process lifetime
- If DNS returns no workers, or no worker returned a cookie list, the controller skips the cycle and keeps previous assignments.
- If `GetStatus` or `ListNodeCookies` fails for a worker, the controller keeps that worker's existing assignments but does not assign new cookies to it during that cycle.

If you change cookie file layout, node cookie semantics, or assignment policy, update both this file and the cookie-assignment code.

## Bot Interactor Constraints

When changing bot logic:

- keep top-level interactor names stable unless there is a strong reason not to
- keep handlers thin and transport-focused
- prefer changing internals in top-level interactors, services, and router layers instead of pushing orchestration back into handlers
- keep quiet-mode behavior structurally separate when it has different presentation rules
- keep any new work that must survive a restart inside a `DownloadJob`, not in memory

## Error Message Style

Static error and status messages should use capitalized sentence case.

Use:

- `Invalid token`
- `Node is at capacity`
- `File exceeds max file size`

Avoid variants such as `invalid token`.

## Build, Lint, and Test

- Prefer `cargo check -p <crate>` for iteration; full `cargo clippy --all-targets` and `cargo test` across the workspace are heavy.
- `just lint` runs clippy with `clippy::pedantic`; `just fmt` uses nightly `rustfmt` with the repo `rustfmt.toml`.
- Bot database tests use `testcontainers` to start PostgreSQL and apply `migration`, so they need a working Docker socket.
- Tests must use synthetic fixtures. Never copy production domains, ids, or titles into tests.
- Do not name custom entity fields after keys `yt-dlp` or `gallery-dl` already emit (for example `direct`); they collide when the raw info JSON is merged.

## Known Limitations

These are accepted for now. Do not "fix" them in passing without discussing the design.

- Downloader cookie storage is one file per domain in `/tmp/cookies/<domain>.txt`, which is what enforces "at most one cookie per domain per node".
- Interactor input DTOs and `DownloadJob` carry Telegram-shaped identifiers (`chat_id`, `message_id`, `inline_message_id`).
- Inbound transport extraction is handler-side; there is no separate inbound adapter layer.
- Job dedup is best-effort: a worker that crashes after the Telegram send but before the done marker is written can deliver a job twice.
- When snapsave is enabled but unreachable, Instagram/Facebook links without a cookie fail fast with `NOT_FOUND`; only a disabled snapsave falls back to the domain-replace rule.

## Change Rules

When modifying this workspace:

- keep bot and downloader protocol changes in sync
- keep downloader and cookie-assignment RPC expectations in sync
- keep `DownloadJob` backward compatible with jobs already in the stream
- do not reintroduce cookie-assignment logic into the bot
- keep shared cert-manager bootstrap resources in the infra chart, not an app chart
- add new user-facing text to all locale files
- update this file if architecture or invariants change

## First Places To Read

If you are making download-related changes, start here:

- [proto/proto/downloader.proto](proto/proto/downloader.proto)
- [downloader_client/src/router.rs](downloader_client/src/router.rs)
- [downloader_client/src/client.rs](downloader_client/src/client.rs)
- [downloader_client/src/retry.rs](downloader_client/src/retry.rs)
- [downloader_client/src/media_info.rs](downloader_client/src/media_info.rs)
- [downloader_client/src/download.rs](downloader_client/src/download.rs)
- [downloader_client/src/resolve.rs](downloader_client/src/resolve.rs)
- [bot/src/services/get_media.rs](bot/src/services/get_media.rs)
- [bot/src/services/download/media.rs](bot/src/services/download/media.rs)
- [downloader/src/grpc/downloader.rs](downloader/src/grpc/downloader.rs)
- [downloader/src/grpc/capabilities.rs](downloader/src/grpc/capabilities.rs)
- [downloader/src/grpc/cookie_manager.rs](downloader/src/grpc/cookie_manager.rs)
- [downloader/src/services/ytdl.rs](downloader/src/services/ytdl.rs)
- [downloader/src/services/snapsave.rs](downloader/src/services/snapsave.rs)
- [downloader/src/services/spotdl.rs](downloader/src/services/spotdl.rs)
- [bot/src/services.rs](bot/src/services.rs)
- [bot/src/interactors/video.rs](bot/src/interactors/video.rs)
- [bot/src/interactors/audio.rs](bot/src/interactors/audio.rs)
- [bot/src/interactors/auto.rs](bot/src/interactors/auto.rs)
- [bot/src/interactors/chosen_inline.rs](bot/src/interactors/chosen_inline.rs)
- [bot/src/interactors/inline_query.rs](bot/src/interactors/inline_query.rs)
- [bot/src/di_container.rs](bot/src/di_container.rs)

If you are making queue or worker changes, start here:

- [bot/src/entities/download_job.rs](bot/src/entities/download_job.rs)
- [bot/src/services/queue.rs](bot/src/services/queue.rs)
- [bot/src/worker.rs](bot/src/worker.rs)
- [bot/src/interactors/enqueue_download.rs](bot/src/interactors/enqueue_download.rs)
- [charts/bot/templates/valkey-cluster.yaml](charts/bot/templates/valkey-cluster.yaml)

If you are making song recognition changes, start here:

- [bot/src/interactors/shazam.rs](bot/src/interactors/shazam.rs)
- [bot/src/services/file_download.rs](bot/src/services/file_download.rs)
- [downloader_client/src/recognize.rs](downloader_client/src/recognize.rs)
- [downloader/src/grpc/song_recognizer.rs](downloader/src/grpc/song_recognizer.rs)
- [downloader/src/services/songrec.rs](downloader/src/services/songrec.rs)

If you are making PostgreSQL backup or object-storage chart changes, start here:

- [charts/bot/templates/postgres-cluster.yaml](charts/bot/templates/postgres-cluster.yaml)
- [charts/bot/templates/postgres-backup-object-store.yaml](charts/bot/templates/postgres-backup-object-store.yaml)
- [charts/bot/templates/postgres-scheduled-backup.yaml](charts/bot/templates/postgres-scheduled-backup.yaml)
- [charts/bot/templates/rustfs-stateful-set.yaml](charts/bot/templates/rustfs-stateful-set.yaml)
- [charts/bot/templates/rustfs-service.yaml](charts/bot/templates/rustfs-service.yaml)
- [charts/bot/templates/rustfs-bucket-bootstrap-job.yaml](charts/bot/templates/rustfs-bucket-bootstrap-job.yaml)

If you are making cookie-assignment changes, start here:

- [cookie_assignment/src/main.rs](cookie_assignment/src/main.rs)
- [cookie_assignment/src/service.rs](cookie_assignment/src/service.rs)
- [cookie_assignment/src/cookies.rs](cookie_assignment/src/cookies.rs)
- [downloader_client/src/cookie_assignment.rs](downloader_client/src/cookie_assignment.rs)
- [scripts/sync-cookies-secret.sh](scripts/sync-cookies-secret.sh)
- [charts/cookie-assignment/templates/cookie-assignment-deployment.yaml](charts/cookie-assignment/templates/cookie-assignment-deployment.yaml)
