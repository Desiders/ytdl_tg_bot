# ytdl_tg_bot Workspace Guide

## Purpose

This workspace contains a Telegram bot, a shared downloader-client crate, a gRPC downloader service, and a separate cookie-assignment controller.

- `bot`: orchestrates requests, caches metadata, routes downloads to downloader nodes, and uploads media to Telegram
- `downloader_client`: shared downloader-node discovery, mTLS client setup, routing, and downloader failover primitives
- `downloader`: runs `yt-dlp`, fetches thumbnails, optionally embeds thumbnails, and streams results back over gRPC
- `cookie_assignment`: discovers downloader nodes and pushes cookie files to them over gRPC
- `proto`: shared protobuf definitions used by the runtime services

This file describes the current architecture and the rules future changes should follow. It is not an implementation plan.

## Workspace Layout

- root workspace members: `bot`, `cookie_assignment`, `downloader_client`, `downloader`, `proto`
- root workspace excludes: `migration`
- crate names:
  - `ytdl_tg_bot`
  - `ytdl_tg_cookie_assignment`
  - `ytdl_tg_downloader_client`
  - `ytdl_tg_downloader`
  - `ytdl_tg_bot_proto`
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
- bot runtime
- downloader nodes
- cookie assignment

Install the infra chart before the app charts so `ca-issuer` exists before certificate resources are reconciled.

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

### Cookie-assignment Controller

- reads cookie files mounted into its own pod
- discovers downloader nodes through the headless downloader service DNS
- checks node availability with `NodeCapabilities.GetStatus`
- lists node cookies with `NodeCookieManager.ListNodeCookies`
- removes stale unassigned cookies from nodes
- pushes free cookies to eligible nodes with `NodeCookieManager.PushCookie`
- keeps assignments in memory only

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

Keep download node selection and downloader-node failover centralized in [`downloader_client/src/router.rs`](/workspace/downloader_client/src/router.rs) and [`downloader_client/src/retry.rs`](/workspace/downloader_client/src/retry.rs). The bot may re-export that crate locally, but the shared downloader access logic should have one source of truth.

The cookie-assignment controller may do its own worker iteration for assignment. Do not reintroduce cookie ownership into the bot.

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
  2. filters nodes that answer status RPCs
  3. loads each node's current cookie IDs
  4. releases stale in-memory assignments
  5. removes stale unassigned cookies from nodes
  6. assigns free cookie files to eligible nodes
  7. pushes cookie content via `NodeCookieManager.PushCookie`
- Downloader nodes remain passive:
  - they never pull cookies
  - they clear `/tmp/cookies/` at startup
  - they keep cookies only for process lifetime

If you change cookie file layout, node cookie semantics, or assignment policy, update both this file and the cookie-assignment code.

## Bot Interactor Constraints

Handlers depend on the current interactor interfaces.

When changing bot download or media-info logic:

- keep public interactor names stable unless there is a strong reason not to
- keep handler-facing input and output types stable when possible
- prefer changing internals in interactors and router instead of touching handlers

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
- [bot/src/interactors/get_media.rs](/workspace/bot/src/interactors/get_media.rs)
- [bot/src/interactors/download/media.rs](/workspace/bot/src/interactors/download/media.rs)
- [downloader/src/grpc/downloader.rs](/workspace/downloader/src/grpc/downloader.rs)
- [downloader/src/grpc/capabilities.rs](/workspace/downloader/src/grpc/capabilities.rs)
- [downloader/src/grpc/cookie_manager.rs](/workspace/downloader/src/grpc/cookie_manager.rs)
- [downloader/src/services/ytdl.rs](/workspace/downloader/src/services/ytdl.rs)

If you are making cookie-assignment changes, start here:

- [cookie_assignment/src/main.rs](/workspace/cookie_assignment/src/main.rs)
- [cookie_assignment/src/service.rs](/workspace/cookie_assignment/src/service.rs)
- [cookie_assignment/src/node_client.rs](/workspace/cookie_assignment/src/node_client.rs)
- [charts/cookie-assignment/templates/cookie-assignment-deployment.yaml](/workspace/charts/cookie-assignment/templates/cookie-assignment-deployment.yaml)
