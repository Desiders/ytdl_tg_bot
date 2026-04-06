# ytdl_tg_bot Workspace Guide

## Purpose

This workspace contains a Telegram bot and a separate gRPC downloader service.

- `bot`: orchestrates requests, caches metadata, routes downloads to nodes, and uploads media to Telegram
- `downloader`: runs yt-dlp, fetches thumbnails, embeds thumbnails when possible, and streams results back over gRPC
- `proto`: shared protobuf definitions used by both crates

This file describes the current architecture and the rules future changes should follow. It is not an implementation plan.

## Workspace Layout

- root workspace members: `bot`, `downloader`, `proto`
- root workspace excludes: `migration`
- crate names:
  - `ytdl_tg_bot`
  - `ytdl_downloader`
  - `ytdl_tg_bot_proto`

## Kubernetes Deployment & Infrastructure

The project is deployed to Kubernetes using Helm charts located in `charts/bot` and `charts/downloader`.

### Service Discovery (Dynamic Nodes)
The bot does **not** use hardcoded IP addresses or static configuration files to find downloader nodes.
- Nodes are discovered dynamically at runtime via Kubernetes Headless Service DNS.
- The bot must perform a standard DNS A-record lookup for `downloader.<namespace>.svc.cluster.local`.
- This lookup returns an array of active pod IPs. The bot passes this array to `NodeRouter`.

### Mutual TLS (mTLS) Constraints
All internal bot-to-node communication uses strict mutual TLS.
- Certificates are automatically provisioned and rotated by `cert-manager` using an internal CA.
- Certificates and keys are mounted into containers at **exact hardcoded paths**. Do not change these paths without updating Helm templates.

**Bot TLS paths:**
- CA: `/app/tls/ca.crt`
- Cert: `/app/tls/bot.crt`
- Key: `/app/tls/bot.key`

**Downloader TLS paths:**
- CA: `/app/tls/ca.crt`
- Cert: `/app/tls/node.crt`
- Key: `/app/tls/node.key`

**CRITICAL TLS RULE (SNI / ServerName):**
Because the bot connects to nodes by IP address (resolved from DNS), but the node's certificate only contains the DNS name in its SAN field, the bot's gRPC TLS client **MUST** explicitly set the `server_name` (SNI) parameter to `downloader.<namespace>.svc.cluster.local`. Failure to do this will result in a TLS handshake failure (`certificate verify failed`).

## Current Download Flow

The bot does not run yt-dlp directly.

1. A handler calls a bot interactor.
2. The bot resolves active downloader node IPs via Kubernetes DNS.
3. The interactor asks `NodeRouter` for a downloader node from that list.
4. The bot connects to the chosen node over gRPC using mTLS (respecting the `server_name` rule).
5. The downloader fetches metadata or downloads media.
6. The downloader streams thumbnail and media bytes back to the bot.
7. The bot forwards media and thumbnail streams to Telegram.

## Current Stream Contract

Proto file: [proto/proto/downloader.proto](/proto/proto/downloader.proto)

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

Do not change this contract casually. If you change it, update both bot and downloader together.

## Thumbnail Rules

This is an important current invariant.

- The downloader may embed the thumbnail into the final media file.
- The downloader also streams the thumbnail separately to the bot.
- The bot must pass the thumbnail to Telegram as a stream.
- Do not save the thumbnail to a temp file just to upload it.
- `MediaForUpload` uses `thumb_stream`, not `thumb_path`.

## Downloader Behavior

1. download thumbnail
2. send `meta`
3. stream `thumbnail_data`
4. download media
5. stream media `data`

## Node Routing

Important behavior:

- The input list of nodes is dynamically resolved via DNS, not read from a static config file.
- prefer nodes with cookies for the target domain
- avoid nodes already at capacity
- retry other nodes on `RESOURCE_EXHAUSTED` and `UNAVAILABLE`
- treat `UNAUTHENTICATED` as a configuration error and surface it as node unavailability to users

Keep routing policy centralized in `NodeRouter`. Do not duplicate node-picking logic across interactors.

## Cookie Lifecycle

- Bot cookies are mounted from Kubernetes Secret under `/app/cookies/<domain>/<n>.txt`.
- Bot reads cookie files recursively from `/app/cookies` during assignment cycles.
- Cookie assignments are in-memory only (no DB persistence).
- The bot runs a background assignment loop:
  1. poll workers
  2. release in-memory assignments for unavailable workers
  3. assign free cookie files to free workers
  4. push cookie content to worker via `NodeCookieManager.PushCookie`
- Workers are passive:
  - they never pull cookies on their own
  - they clear `/tmp/cookies/` at startup
  - they keep cookies only in `/tmp/cookies/` for process lifetime

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
- update this file if architecture or invariants change

## First Places To Read

If you are making download-related changes, start here:

- [proto/proto/downloader.proto](/proto/proto/downloader.proto)
- [bot/src/services/node_router.rs](/bot/src/services/node_router.rs)
- [bot/src/interactors/get_media.rs](/bot/src/interactors/get_media.rs)
- [bot/src/interactors/download/media.rs](/bot/src/interactors/download/media.rs)
- [bot/src/services/cookie_assignment.rs](/bot/src/services/cookie_assignment.rs)
- [downloader/src/grpc/downloader.rs](/downloader/src/grpc/downloader.rs)
- [downloader/src/grpc/cookie_manager.rs](/downloader/src/grpc/cookie_manager.rs)
- [downloader/src/services/ytdl.rs](/downloader/src/services/ytdl.rs)
