# ytdl_tg_bot Workspace Guide

## Purpose

This workspace contains a Telegram bot, a shared downloader-client crate, a gRPC downloader service, and a separate cookie-assignment controller.

- `bot`: orchestrates requests, caches metadata, routes downloads to downloader nodes, and uploads media to Telegram
- `downloader_client`: shared downloader-node discovery, mTLS client setup, routing, downloader failover, and downloader RPC adapter logic
- `downloader`: runs `yt-dlp`, fetches thumbnails, optionally embeds thumbnails, and streams results back over gRPC
- `cookie_assignment`: discovers downloader nodes and pushes cookie files to them over gRPC
- `proto`: shared protobuf definitions used by the runtime services

This file describes the current architecture and the rules future changes should follow. It is not an implementation plan.

## Workspace Layout

- root workspace members: `bot`, `cookie_assignment`, `downloader_client`, `downloader`, `proto`
- root workspace excludes: `migration`
- crate names:
  - `bot`
  - `cookie_assignment`
  - `downloader_client`
  - `downloader`
  - `proto`
- helm charts:
  - `charts/infra`
  - `charts/bot`
  - `charts/cookie-assignment`
  - `charts/downloader`
- local config templates:
  - `configs/config.toml`
  - `configs/cookie_assignment.toml`
  - `configs/downloader.toml`

## Kubernetes Deployment & Infrastructure

The project is deployed to Kubernetes with separate Helm charts for:

- shared infrastructure
- bot runtime, PostgreSQL, internal RustFS backup storage, and PostgreSQL scheduled backup resources
- downloader nodes
- cookie assignment

Install the infra chart before the app charts so `ca-issuer` exists before certificate resources are reconciled. CloudNativePG `1.26+` and the Barman Cloud CNPG-I Plugin must already be installed before the bot chart is installed. Prefer CloudNativePG `1.29+` for new deployments.

### PostgreSQL Backup

The bot chart uses the current CloudNativePG plugin-based backup path.

- `charts/bot` creates a `barmancloud.cnpg.io/v1` `ObjectStore`.
- The `postgresql.cnpg.io/v1` `Cluster` references that object store through `spec.plugins`.
- `ScheduledBackup` uses `method: plugin` with `barman-cloud.cloudnative-pg.io`.
- Do not reintroduce `Cluster.spec.backup.barmanObjectStore`; that is the deprecated in-tree Barman Cloud path.
- CNPG backup schedules use six-field cron syntax with seconds first, for example `0 0 3 * * *`.
- The default chart includes single-node RustFS and bootstraps the `backups` bucket through a Helm hook Job.
- If RustFS is disabled and an external S3-compatible store is used, the bucket must exist before backups run.

### Service Discovery (Dynamic Nodes)

The bot and the cookie-assignment controller do **not** use hardcoded node IPs or static node lists.

- Downloader nodes are discovered dynamically via Kubernetes Headless Service DNS.
- Clients must resolve `downloader.<namespace>.svc.cluster.local`.
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

Both the bot and the cookie-assignment controller connect to nodes by IP address, but the downloader certificate SAN contains the DNS name. The gRPC TLS client **MUST** set `server_name` / SNI to `downloader.<namespace>.svc.cluster.local`.

If this rule is broken, the TLS handshake fails with certificate verification errors.

## Current Runtime Roles

### Bot

- accepts Telegram updates
- fetches media metadata through downloader nodes
- selects nodes through `NodeRouter`
- downloads media streams from downloader nodes
- uploads media and thumbnail streams to Telegram
- refreshes node status periodically
- refreshes downloader capabilities from node-reported cookie domains

### Bot Messenger Boundary

- Outbound messenger operations should go through `MessengerPort`.
- `TelegramMessenger` is the adapter that owns Telegram API calls, retries, parse modes, inline answer wiring, and Telegram media send/edit method construction.
- Bot handlers, top-level bot interactors, and `send_media` interactors should depend on the messenger port layer, not construct Telegram methods directly.
- Keep Telegram SDK/API types isolated to the Telegram adapter. Utility string helpers such as HTML escaping may still live elsewhere, but Telegram request construction should have one source of truth.

### Bot Handler / Interactor Boundary

- Handlers are Telegram inbound adapters.
- A handler should:
  - extract Telegram input
  - inject one top-level interactor
  - call it
  - return `EventReturn::Finish`
- Do not let handlers orchestrate business flow across multiple top-level interactors or services again.
- Handler-facing orchestration lives in `bot/src/interactors/`.
- Lower-level reusable building blocks live under `bot/src/services/`, not in the interactor namespace.
- Current service modules used by interactors are:
  - `chat`
  - `download`
  - `downloaded_media`
  - `get_media`
  - `node_router`
  - `send_media`
- Top-level interactors may call these services.
- Services must not call top-level interactors.

### Bot DI Style

- Keep the current generic DI style around `Messenger`.
- The composition root builds `TelegramMessenger`, but top-level interactors should be wired generically over `Messenger` rather than directly against the concrete adapter type.
- If you add a new top-level interactor, register it in `interactors_registry<Messenger>(...)` and keep the same generic pattern.

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

- `Downloader` accepts any token listed in downloader config `[auth].node_tokens`.
- `NodeCapabilities` accepts any token listed in `[auth].node_tokens` or the cookie-manager token.
- `NodeCookieManager` uses the cookie-manager token.
- Each bot config must only contain one normal node token, and that token must be present in downloader `[auth].node_tokens`.
- The cookie-assignment config must only contain the cookie-manager token.
- Do not give the bot the cookie-manager token.

### Adding Another Bot

New messenger bots should be separate applications, not modes inside the Telegram bot.

- Add a new crate such as `bot_discord` when another messenger is needed.
- The new bot should depend on `downloader_client` directly for downloader DNS discovery, mTLS setup, node routing, failover, media info, and download streams.
- Do not depend on the existing `bot` crate from another bot.
- Add an independent chart for the new bot. Copying `charts/bot` as a starting point is acceptable, but remove resources that are not needed by the new bot.
- The current `charts/bot` PostgreSQL, RustFS, migrations, Telegram Bot API, yt-toolkit, and upload cache resources are Telegram-bot runtime choices, not mandatory shared infrastructure for every bot.
- Another bot may be simple and have no database or upload cache.
- If another bot needs a cache, design it around that messenger's remote media identifiers. Do not reuse Telegram `file_id` semantics as shared state.
- Each bot must have its own config Secret and client TLS certificate.
- Each bot config should contain exactly one normal downloader token, and downloader config `[auth].node_tokens` must include that token.
- Never give a bot the cookie-manager token.
- Keep downloader nodes and cookie assignment shared only through `downloader_client`, gRPC, mTLS, and Kubernetes service discovery.

### Downloader Node

- exposes `Downloader`, `NodeCapabilities`, and `NodeCookieManager` gRPC services
- stores assigned cookies only in `/tmp/cookies`
- clears `/tmp/cookies` on startup
- reports supported domains from currently assigned in-memory cookie state
- never pulls cookies on its own

## Current Download Flow

The bot does not run `yt-dlp` directly.

1. A handler calls a bot interactor.
2. The bot resolves active downloader node IPs via Kubernetes DNS.
3. The interactor asks `NodeRouter` for a downloader node.
4. The bot connects to the chosen node over gRPC using mTLS and the downloader DNS name as SNI.
5. The downloader fetches metadata or downloads media.
6. The downloader streams thumbnail and media bytes back to the bot.
7. The bot forwards media and thumbnail streams to Telegram.

## Current Stream Contract

Proto file: [proto/proto/downloader.proto](/workspace/proto/proto/downloader.proto)

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

1. `meta`
2. zero or more `thumbnail_data`
3. zero or more `progress`
4. one or more `data`

Do not change this casually. If you change it, update bot and downloader together.

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
- prefer nodes with cookies for the target domain
- avoid nodes already at capacity
- retry other nodes on `RESOURCE_EXHAUSTED` and `UNAVAILABLE`
- treat `UNAUTHENTICATED` as a configuration error and surface it as node unavailability to users
- treat retryable `ABORTED` downloader responses as node-context failures and retry other nodes first

Keep download node selection, downloader-node failover, and downloader RPC adapter logic centralized in:

- [`downloader_client/src/router.rs`](/workspace/downloader_client/src/router.rs)
- [`downloader_client/src/retry.rs`](/workspace/downloader_client/src/retry.rs)
- [`downloader_client/src/media_info.rs`](/workspace/downloader_client/src/media_info.rs)
- [`downloader_client/src/download.rs`](/workspace/downloader_client/src/download.rs)
- [`downloader_client/src/cookie_assignment.rs`](/workspace/downloader_client/src/cookie_assignment.rs)

The bot may re-export that crate locally, and the cookie-assignment controller may keep its own assignment policy, but DNS discovery, mTLS channel setup, auth request construction, and downloader RPC adapters should stay in `downloader_client`.

## Cookie Lifecycle

The bot no longer owns cookie distribution.

- Cookie files are mounted only into the cookie-assignment deployment.
- Source cookie layout in the repo is `cookies/<domain>/<cookie-id>.txt`.
- Mounted cookie entry names are flattened as `<domain>__<cookie-id>.txt`.
- The cookie-assignment runtime reads flattened `*.txt` files from `/app/cookies` and decodes them back into domain plus file name.
- `cookie_id` is tracked as `<domain>/<cookie-id>.txt` so file names may repeat across domains without colliding.
- Cookie assignments are in-memory only inside the cookie-assignment controller.
- The controller assignment cycle currently:
  1. resolves downloader nodes from DNS
  2. checks node status through `NodeCapabilities.GetStatus` using the cookie-manager token
  3. loads each node's current cookie IDs through `NodeCookieManager.ListNodeCookies`
  4. releases stale in-memory assignments
  5. removes stale unassigned cookies from nodes
  6. assigns free cookie files to eligible nodes
  7. pushes cookie content via `NodeCookieManager.PushCookie`
- Downloader nodes remain passive:
  - they never pull cookies
  - they clear `/tmp/cookies/` at startup
  - they keep cookies only for process lifetime
- If DNS returns no workers, the controller skips the cycle and keeps previous assignments.
- If `GetStatus` fails for a worker, the controller keeps that worker's existing assignments but does not assign new cookies to it during that cycle.
- If `ListNodeCookies` fails for a worker, the controller keeps that worker's existing assignments but does not assign new cookies to it during that cycle.

If you change cookie file layout, node cookie semantics, or assignment policy, update both this file and the cookie-assignment code.

## Bot Interactor Constraints

When changing bot logic:

- keep top-level interactor names stable unless there is a strong reason not to
- keep handlers thin and transport-focused
- prefer changing internals in top-level interactors, services, and router layers instead of pushing orchestration back into handlers
- keep quiet-mode behavior structurally separate when it has different presentation rules

## Error Message Style

Static error and status messages should use capitalized sentence case.

Use:

- `Invalid token`
- `Node is at capacity`
- `File exceeds max file size`

Avoid variants such as `invalid token`.

## Change Rules

When modifying this workspace:

- keep bot and downloader protocol changes in sync
- keep downloader and cookie-assignment RPC expectations in sync
- do not reintroduce cookie-assignment logic into the bot
- keep shared cert-manager bootstrap resources in the infra chart, not an app chart
- update this file if architecture or invariants change

## First Places To Read

If you are making download-related changes, start here:

- [proto/proto/downloader.proto](/workspace/proto/proto/downloader.proto)
- [downloader_client/src/router.rs](/workspace/downloader_client/src/router.rs)
- [downloader_client/src/client.rs](/workspace/downloader_client/src/client.rs)
- [downloader_client/src/retry.rs](/workspace/downloader_client/src/retry.rs)
- [downloader_client/src/media_info.rs](/workspace/downloader_client/src/media_info.rs)
- [downloader_client/src/download.rs](/workspace/downloader_client/src/download.rs)
- [bot/src/services/get_media.rs](/workspace/bot/src/services/get_media.rs)
- [bot/src/services/download/media.rs](/workspace/bot/src/services/download/media.rs)
- [downloader/src/grpc/downloader.rs](/workspace/downloader/src/grpc/downloader.rs)
- [downloader/src/grpc/capabilities.rs](/workspace/downloader/src/grpc/capabilities.rs)
- [downloader/src/grpc/cookie_manager.rs](/workspace/downloader/src/grpc/cookie_manager.rs)
- [downloader/src/services/ytdl.rs](/workspace/downloader/src/services/ytdl.rs)
- [bot/src/services.rs](/workspace/bot/src/services.rs)
- [bot/src/interactors/video.rs](/workspace/bot/src/interactors/video.rs)
- [bot/src/interactors/audio.rs](/workspace/bot/src/interactors/audio.rs)
- [bot/src/interactors/chosen_inline.rs](/workspace/bot/src/interactors/chosen_inline.rs)
- [bot/src/interactors/inline_query.rs](/workspace/bot/src/interactors/inline_query.rs)
- [bot/src/di_container.rs](/workspace/bot/src/di_container.rs)

If you are making PostgreSQL backup or object-storage chart changes, start here:

- [charts/bot/templates/postgres-cluster.yaml](/workspace/charts/bot/templates/postgres-cluster.yaml)
- [charts/bot/templates/postgres-backup-object-store.yaml](/workspace/charts/bot/templates/postgres-backup-object-store.yaml)
- [charts/bot/templates/postgres-scheduled-backup.yaml](/workspace/charts/bot/templates/postgres-scheduled-backup.yaml)
- [charts/bot/templates/rustfs-stateful-set.yaml](/workspace/charts/bot/templates/rustfs-stateful-set.yaml)
- [charts/bot/templates/rustfs-service.yaml](/workspace/charts/bot/templates/rustfs-service.yaml)
- [charts/bot/templates/rustfs-bucket-bootstrap-job.yaml](/workspace/charts/bot/templates/rustfs-bucket-bootstrap-job.yaml)

If you are making cookie-assignment changes, start here:

- [cookie_assignment/src/main.rs](/workspace/cookie_assignment/src/main.rs)
- [cookie_assignment/src/service.rs](/workspace/cookie_assignment/src/service.rs)
- [downloader_client/src/cookie_assignment.rs](/workspace/downloader_client/src/cookie_assignment.rs)
- [charts/cookie-assignment/templates/cookie-assignment-deployment.yaml](/workspace/charts/cookie-assignment/templates/cookie-assignment-deployment.yaml)
