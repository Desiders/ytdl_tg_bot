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
  - `ytdl_tg_downloader`
  - `ytdl_tg_bot_proto`

## Current Download Flow

The bot does not run yt-dlp directly.

1. A handler calls a bot interactor.
2. The interactor asks `NodeRouter` for a downloader node.
3. The bot calls the downloader over gRPC.
4. The downloader fetches metadata or downloads media.
5. The downloader streams thumbnail and media bytes back to the bot.
6. The bot forwards media and thumbnail streams to Telegram.

Bot-to-node transport may now be plaintext or TLS depending on node address and config.
- `http://...` nodes use plaintext gRPC.
- `https://...` nodes use TLS and must be configured with a CA on the bot side.
- Mixed `http://` and `https://` nodes are supported at the same time.

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

Relevant files:

- [downloader/src/grpc/downloader.rs](/downloader/src/grpc/downloader.rs)
- [bot/src/interactors/download/media.rs](/bot/src/interactors/download/media.rs)
- [bot/src/interactors/send_media/upload.rs](/bot/src/interactors/send_media/upload.rs)
- [bot/src/entities/media.rs](/bot/src/entities/media.rs)

## Downloader Behavior

Current downloader behavior is intentionally sequential.

1. download thumbnail
2. send `meta`
3. stream `thumbnail_data`
4. download media
5. stream media `data`

There was discussion about making thumbnail and media work concurrent, but the current code does not do that. Do not describe it as concurrent unless the implementation actually changes.

## Node Routing

Node selection lives in [bot/src/node_router.rs](/bot/src/node_router.rs).

Important behavior:

- prefer nodes with cookies for the target domain
- avoid nodes already at capacity
- retry other nodes on `RESOURCE_EXHAUSTED` and `UNAVAILABLE`
- treat `UNAUTHENTICATED` as a configuration error and surface it as node unavailability to users

Keep routing policy centralized in `NodeRouter`. Do not duplicate node-picking logic across interactors.

## Bot Interactor Constraints

Handlers depend on the current interactor interfaces.

When changing bot download or media-info logic:

- keep public interactor names stable unless there is a strong reason not to
- keep handler-facing input and output types stable when possible
- prefer changing internals in interactors and router instead of touching handlers

Files that are tightly coupled to this:

- [bot/src/interactors/get_media.rs](/bot/src/interactors/get_media.rs)
- [bot/src/interactors/download/media.rs](/bot/src/interactors/download/media.rs)
- [bot/src/handlers](/bot/src/handlers)

## Error Message Style

Static error and status messages should use capitalized sentence case.

Use:

- `Invalid token`
- `Node is at capacity`
- `File exceeds max file size`

Avoid lowercase variants such as `invalid token`.

## Build Notes

The `proto` crate requires `protoc` at build time.

- The repo does not currently vendor `protoc`.
- Local builds therefore need either system `protoc` in `PATH` or `PROTOC` set explicitly.

## Change Rules

When modifying this workspace:

- keep bot and downloader protocol changes in sync
- do not reintroduce direct yt-dlp usage into the bot
- do not reintroduce thumbnail temp-file upload flow on the bot side
- update this file if architecture or invariants change

## First Places To Read

If you are making download-related changes, start here:

- [proto/proto/downloader.proto](/proto/proto/downloader.proto)
- [bot/src/node_router.rs](/bot/src/node_router.rs)
- [bot/src/interactors/get_media.rs](/bot/src/interactors/get_media.rs)
- [bot/src/interactors/download/media.rs](/bot/src/interactors/download/media.rs)
- [downloader/src/grpc/downloader.rs](/downloader/src/grpc/downloader.rs)
- [downloader/src/services/ytdl.rs](/downloader/src/services/ytdl.rs)
