# Distributed Download Nodes — Agent Implementation Guide

## Context

This is an existing Rust Telegram bot (`ytdl_tg_bot`) that downloads media via yt-dlp and sends it to users.
The codebase is a single Cargo crate. The goal is to split download functionality into separate gRPC services
called **download nodes**, while the bot server becomes an orchestrator.

Read the existing codebase fully before starting. Every task below references specific existing files and types.

---

## Workspace Setup

### Task 0 — Convert to Cargo Workspace

**Do this first. Everything else depends on it.**

#### Create `/Cargo.toml` (workspace root, replaces current `Cargo.toml`)

```toml
[workspace]
members = [
    "bot",
    "downloader",
    "proto",
]
resolver = "2"
```

#### Move existing crate into `bot/`

- Move all existing `src/` into `bot/src/`
- Move existing `Cargo.toml` into `bot/Cargo.toml`
- Change `bot/Cargo.toml` `[package].name` to `"ytdl_tg_bot_bot"`

#### Create `proto/` crate

```
proto/
  Cargo.toml
  build.rs
  proto/
    downloader.proto
```

`proto/Cargo.toml`:
```toml
[package]
name = "ytdl_tg_bot_proto"
version = "0.1.0"
edition = "2021"

[dependencies]
tonic = { version = "0.12", default-features = false }
prost = "0.13"

[build-dependencies]
tonic-build = "0.12"
```

`proto/build.rs`:
```rust
fn main() {
    tonic_build::compile_protos("proto/downloader.proto").unwrap();
}
```

#### Create `downloader/` crate

```
downloader/
  Cargo.toml
  src/
    main.rs
```

`downloader/Cargo.toml`:
```toml
[package]
name = "ytdl_tg_bot_downloader"
version = "0.1.0"
edition = "2021"

[dependencies]
ytdl_tg_bot_proto = { path = "../proto" }
tokio = { version = "1.36", features = ["rt-multi-thread", "process", "sync"], default-features = false }
tonic = { version = "0.12", features = ["transport"] }
# copy relevant deps from bot: nix, tracing, tracing-subscriber, thiserror, tempfile,
# tokio-util, futures-util, serde, serde_json, bytes, backoff, toml, anyhow, mutagen-rs
```

Add to `bot/Cargo.toml`:
```toml
ytdl_tg_bot_proto = { path = "../proto" }
tonic = { version = "0.12", features = ["transport"] }
```

**Acceptance criteria:** `cargo build --workspace` compiles with empty `downloader/src/main.rs` (just `fn main() {}`).

---

## Task 1 — Define the Proto File

**File to create:** `proto/proto/downloader.proto`

```protobuf
syntax = "proto3";
package downloader;

// ─── Shared ────────────────────────────────────────────────────────────────

message Empty {}

message Range {
  int32 start = 1;
  int32 count = 2;
  int32 step  = 3;
}

message Section {
  optional int32 start = 1;
  optional int32 end   = 2;
}

// ─── Media Info ────────────────────────────────────────────────────────────

message MediaInfoRequest {
  string url            = 1;
  string audio_language = 2; // empty = default
  Range  playlist_range = 3;
  string media_type     = 4; // "video" or "audio"
}

message MediaFormatEntry {
  string   format_id       = 1;
  string   ext             = 2;
  optional int64  width    = 3;
  optional int64  height   = 4;
  optional float  aspect_ratio    = 5;
  optional uint64 filesize_approx = 6;
  string   raw_info_json   = 7; // full yt-dlp JSON line for this entry
}

message MediaEntry {
  string   id             = 1;
  optional string display_id     = 2;
  string   webpage_url    = 3;
  optional string title          = 4;
  optional string uploader       = 5;
  optional float  duration       = 6;
  int32    playlist_index = 7;
  optional string domain         = 8;
  optional string audio_language = 9; // resolved language
  repeated MediaFormatEntry formats = 10;
}

message MediaInfoResponse {
  repeated MediaEntry entries = 1;
}

// ─── Download ───────────────────────────────────────────────────────────────

message DownloadRequest {
  string   url           = 1;
  string   format_id     = 2;
  string   raw_info_json = 3;
  string   media_type    = 4; // "video" or "audio"
  string   audio_ext     = 5; // e.g. "m4a"; only used when media_type = "audio"
  optional Section section       = 6;
  uint64   max_file_size = 7; // bytes
}

message DownloadMeta {
  string   ext           = 1;
  optional int64 width   = 2;
  optional int64 height  = 3;
  optional int64 duration = 4;
}

message DownloadChunk {
  oneof payload {
    DownloadMeta meta     = 1; // always the first message in the stream
    string       progress = 2; // yt-dlp progress string, interleaved with data
    bytes        data     = 3; // file bytes
  }
}

// ─── Node Capabilities ──────────────────────────────────────────────────────

message NodeStatus {
  uint32 active_downloads = 1;
  uint32 max_concurrent   = 2;
}

message SupportedDomainsResponse {
  repeated string domains_with_cookies = 1;
}

// ─── Services ───────────────────────────────────────────────────────────────

service Downloader {
  rpc GetMediaInfo   (MediaInfoRequest) returns (MediaInfoResponse);
  rpc DownloadMedia  (DownloadRequest)  returns (stream DownloadChunk);
}

service NodeCapabilities {
  rpc GetStatus           (Empty) returns (NodeStatus);
  rpc GetSupportedDomains (Empty) returns (SupportedDomainsResponse);
}
```

**Stream contract for `DownloadMedia`:**
1. First message: `DownloadChunk { meta: DownloadMeta { ... } }`
2. Zero or more: `DownloadChunk { progress: "..." }` (interleaved with data)
3. One or more: `DownloadChunk { data: <bytes> }`
4. Stream close with gRPC status `OK` = success. Any other status = failure.

**Acceptance criteria:** `cargo build -p ytdl_tg_bot_proto` succeeds and generates Rust types.

---

## Task 2 — Download Node: Config

**File to create:** `downloader/src/config.rs`

Mirror the style of `bot/src/config.rs`. Use `serde::Deserialize` and `toml`.

```rust
pub struct ServerConfig {
    pub address: Box<str>,       // e.g. "0.0.0.0:50051"
    pub max_concurrent: u32,
}

pub struct AuthConfig {
    pub tokens: Vec<Box<str>>,   // list allows rotation; validate any matching token
}

pub struct YtDlpConfig {
    pub executable_path: Box<str>,
    pub cookies_path: Box<str>,
    pub max_file_size: u64,      // bytes, secondary safeguard
}

pub struct YtPotProviderConfig {
    pub url: Box<str>,
}

pub struct LoggingConfig {
    pub dirs: Box<str>,
}

pub struct Config {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub yt_dlp: YtDlpConfig,
    pub yt_pot_provider: YtPotProviderConfig,
    pub logging: LoggingConfig,
}
```

Example `downloader.example.toml`:
```toml
[server]
address = "0.0.0.0:50051"
max_concurrent = 5

[auth]
tokens = ["secret-token-a"]

[yt_dlp]
executable_path = "./yt-dlp/executable"
cookies_path = "./cookies"
max_file_size = 5000000

[yt_pot_provider]
url = "http://127.0.0.1:4416"

[logging]
dirs = "info"
```

Config is loaded from the path in env var `DOWNLOADER_CONFIG_PATH`, defaulting to `configs/downloader.toml`.
Panic on missing or malformed config (same pattern as `bot/src/config.rs`).

**Acceptance criteria:** Config parses correctly from the example toml above.

---

## Task 3 — Download Node: Move Existing Services

The download node reuses existing logic from the bot. **Copy** (do not move, the bot may still reference
stubs temporarily) these files into `downloader/src/services/`:

| Source (bot crate) | Destination (downloader crate) |
|---|---|
| `bot/src/services/ytdl.rs` | `downloader/src/services/ytdl.rs` |
| `bot/src/services/mutagen.rs` | `downloader/src/services/mutagen.rs` |
| `bot/src/services/ffmpeg.rs` | `downloader/src/services/ffmpeg.rs` |
| `bot/src/services/fs/cookies.rs` | `downloader/src/services/fs/cookies.rs` |
| `bot/src/entities/cookies.rs` | `downloader/src/entities/cookies.rs` |
| `bot/src/entities/media.rs` | `downloader/src/entities/media.rs` |
| `bot/src/entities/range.rs` | `downloader/src/entities/range.rs` |
| `bot/src/entities/sections.rs` | `downloader/src/entities/sections.rs` |
| `bot/src/entities/language.rs` | `downloader/src/entities/language.rs` |
| `bot/src/value_objects/media_type.rs` | `downloader/src/value_objects/media_type.rs` |

Remove the `sea_orm` dependency from copied files — `downloader` has no database.
The `value_objects/media_type.rs` in the downloader does not need the `From<Model>` / `From<MediaType>`
conversions that reference `sea_orm_active_enums`. Keep only the enum definition.

**Acceptance criteria:** `cargo build -p ytdl_tg_bot_downloader` compiles the copied modules without errors.

---

## Task 4 — Download Node: gRPC Service Implementations

**Files to create:**
- `downloader/src/grpc/capabilities.rs` — implements `NodeCapabilities`
- `downloader/src/grpc/downloader.rs` — implements `Downloader`
- `downloader/src/grpc/auth.rs` — request authentication interceptor

### 4a — Auth Interceptor (`grpc/auth.rs`)

Implement a `tonic` interceptor that:
1. Reads the `authorization` metadata header from every incoming request.
2. Expects the value `Bearer <token>`.
3. Checks the token against `AuthConfig.tokens`.
4. Returns gRPC status `UNAUTHENTICATED` with message `"invalid token"` if missing or invalid.
5. Passes through if valid.

Apply this interceptor to both services in `main.rs`.

### 4b — NodeCapabilities Service (`grpc/capabilities.rs`)

```rust
pub struct CapabilitiesService {
    pub cookies: Arc<Cookies>,
    pub active_downloads: Arc<AtomicU32>,
    pub max_concurrent: u32,
}
```

`GetStatus`:
- Returns `NodeStatus { active_downloads: <current>, max_concurrent: <from config> }`.
- Read `active_downloads` from the shared `AtomicU32`.

`GetSupportedDomains`:
- Returns `SupportedDomainsResponse { domains_with_cookies: cookies.get_hosts()... }`.
- Convert each `Host` to its string representation.

### 4c — Downloader Service (`grpc/downloader.rs`)

```rust
pub struct DownloaderService {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub cookies: Arc<Cookies>,
    pub active_downloads: Arc<AtomicU32>,
    pub semaphore: Arc<Semaphore>, // tokio::sync::Semaphore with max_concurrent permits
}
```

**`GetMediaInfo` implementation:**

1. Parse `MediaInfoRequest` fields into existing types: `Range`, `Language`, `FormatStrategy`.
2. `media_type = "video"` → `FormatStrategy::VideoAndAudio`.
   `media_type = "audio"` → `FormatStrategy::AudioOnly { audio_ext: "m4a" }`.
3. Select cookie via `cookies.get_path_by_optional_host(url.host())`.
4. Call existing `ytdl::get_media_info(...)` (copied in Task 3).
5. Map the resulting `Playlist` into `MediaInfoResponse`:
   - Each `(Media, Vec<(MediaFormat, RawMediaWithFormat)>)` becomes a `MediaEntry`.
   - Each `(MediaFormat, RawMediaWithFormat)` becomes a `MediaFormatEntry` where `raw_info_json` = the raw JSON string.
6. Return `Ok(Response::new(response))`.

**`DownloadMedia` implementation:**

1. Acquire one permit from the semaphore. If `try_acquire` fails → return gRPC status `RESOURCE_EXHAUSTED`
   with message `"node is at capacity"`. Do NOT block waiting for a permit.
2. Increment `active_downloads`.
3. In a `tokio::spawn` (or directly in the async fn), run the download:
   a. Write `raw_info_json` to a temp file.
   b. Create `mpsc::unbounded_channel` for progress strings.
   c. Call existing `ytdl::download_media(...)`.
   d. Stream results back via `tokio_stream`:
      - Send `DownloadChunk { meta: DownloadMeta { ext, width, height, duration } }` first.
      - Send `DownloadChunk { progress: "..." }` as progress strings arrive.
      - After download completes, read the output file in chunks (e.g. 256KB) and send
        `DownloadChunk { data: bytes }` for each.
   e. If `ytdl::download_media` returns an error → close stream with gRPC status `INTERNAL`
      and the error message.
   f. If file exceeds `max_file_size` → abort and return gRPC status `INVALID_ARGUMENT`
      with message `"file exceeds max_file_size"`.
4. Decrement `active_downloads` and release semaphore permit in a `Drop` guard regardless of outcome.

**Acceptance criteria:**
- Node starts, listens on configured address.
- `GetStatus` returns correct values.
- `GetSupportedDomains` returns hosts derived from cookie files.
- Requests without a valid token are rejected with `UNAUTHENTICATED`.
- `GetMediaInfo` returns entries for a valid URL.
- `DownloadMedia` streams a file and the reassembled bytes match the original download.

---

## Task 5 — Download Node: `main.rs`

**File:** `downloader/src/main.rs`

```rust
#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // 1. Load config (Task 2)
    // 2. Init tracing (same pattern as bot/src/main.rs)
    // 3. Load cookies via get_cookies_from_directory (Task 3)
    // 4. Create shared state:
    //      active_downloads: Arc<AtomicU32>
    //      semaphore: Arc<Semaphore::new(config.server.max_concurrent)>
    //      cookies: Arc<Cookies>
    // 5. Build auth interceptor (Task 4a)
    // 6. Build CapabilitiesService and DownloaderService (Task 4b, 4c)
    // 7. Start tonic Server on config.server.address
    //    with both services wrapped in the auth interceptor
    // 8. Await server shutdown
}
```

**Acceptance criteria:** `cargo run -p ytdl_tg_bot_downloader` starts without panicking.

---

## Task 6 — Bot Server: New Config Fields

**File to modify:** `bot/src/config.rs`

### Add new types

```rust
#[derive(Deserialize, Clone, Debug)]
pub struct DownloadNodeConfig {
    pub address: Box<str>,
    pub token: Box<str>,
    pub max_concurrent: u32,
}

#[derive(Default, Deserialize, Clone, Debug)]
pub struct DownloadConfig {
    #[serde(default)]
    pub capabilities_refresh_interval: u64, // seconds; 0 = startup only
    #[serde(default = "default_overload_strategy")]
    pub overload_strategy: OverloadStrategy,
    #[serde(default)]
    pub nodes: Vec<DownloadNodeConfig>,
}

#[derive(Default, Deserialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum OverloadStrategy {
    #[default]
    Reject,
}
```

### Modify `Config` struct

Add:
```rust
pub download: DownloadConfig,
```

Remove from `Config` (these move to the node):
- `pub yt_pot_provider: YtPotProviderConfig`

Keep in `Config`:
- `pub yt_dlp: YtDlpConfig` — but `YtDlpConfig` no longer needs `executable_path` or `cookies_path`.

### Modify `YtDlpConfig`

Remove fields:
- `executable_path`
- `cookies_path`

Keep:
- `max_file_size: u64`

### Update `config.toml` example

Add:
```toml
[download]
capabilities_refresh_interval = 0
overload_strategy = "reject"

[[download.nodes]]
address = "http://node-a.internal:50051"
token = "secret-token-a"
max_concurrent = 5
```

Remove from example:
```toml
# remove these sections entirely:
[yt_pot_provider]
[yt_dlp] executable_path and cookies_path fields
```

**Acceptance criteria:** Bot server config parses with the new fields. Old `yt_pot_provider` and yt-dlp path
fields are gone from the struct (will cause compile errors in Task 7 which you then fix).

---

## Task 7 — Bot Server: Remove Direct yt-dlp Usage

These files call yt-dlp or depend on cookies/pot-provider directly. Replace their internals with gRPC calls
or delete them as described.

### Files to DELETE from `bot/src/`

- `bot/src/services/ytdl.rs`
- `bot/src/services/mutagen.rs`
- `bot/src/services/ffmpeg.rs`
- `bot/src/services/fs/cookies.rs`
- `bot/src/entities/cookies.rs`

Update `bot/src/services.rs` and `bot/src/services/fs.rs` module declarations accordingly.
Update `bot/src/entities.rs` to remove `cookies` module and `pub use cookies::{Cookie, Cookies}`.

### Files to MODIFY

**`bot/src/main.rs`**:
- Remove: `use crate::services::get_cookies_from_directory;`
- Remove: `let cookies = get_cookies_from_directory(...)`
- Remove: `info!(hosts = ?cookies.get_hosts(), "Cookies loaded");`
- Remove: bot/files url construction using `config.telegram_bot_api` (keep the api server setup, just remove
  the cookies part)
- The `container` init call will change in Task 8.

**`bot/src/di_container.rs`**:
- Remove: `instance(cookies)` from registry.
- Remove: `instance(cfg.yt_pot_provider)` from registry.
- Remove all `provide(...)` calls that instantiate:
  - `get_media::GetUncachedVideoByURL`
  - `get_media::GetVideoByURL`
  - `get_media::GetAudioByURL`
  - `download::media::DownloadVideo`
  - `download::media::DownloadAudio`
  - `download::media::DownloadVideoPlaylist`
  - `download::media::DownloadAudioPlaylist`
- These will be replaced in Task 8 with the node router.

**`bot/src/interactors/get_media.rs`**:
- DELETE the entire file content.
- Will be rewritten in Task 9.

**`bot/src/interactors/download/media.rs`**:
- DELETE the entire file content.
- Will be rewritten in Task 9.

**`bot/src/services/fs.rs`** (if it only re-exported cookies):
- Remove cookies re-export. If the module is now empty, delete it.

**Acceptance criteria:** After deletions and stubs, `cargo build -p ytdl_tg_bot_bot` should fail only on
missing types that Tasks 8 and 9 will provide — not on anything related to yt-dlp, cookies, or pot-provider.

---

## Task 8 — Bot Server: Node Router

**File to create:** `bot/src/node_router.rs`

This is the central new component on the bot server. It holds the list of nodes, their current load, and
handles routing decisions.

### Types

```rust
pub struct NodeHandle {
    pub address: Box<str>,
    pub token: Box<str>,
    pub max_concurrent: u32,
    pub active_downloads: AtomicU32,  // optimistic counter managed by bot server
    // gRPC channel, lazily or eagerly connected
    pub channel: tonic::transport::Channel,
}

pub struct NodeRouter {
    nodes: Vec<Arc<NodeHandle>>,
    // domain -> indices into `nodes` that have cookies for that domain
    domain_cookie_map: HashMap<String, Vec<usize>>,
    max_file_size: u64,
}
```

### `NodeRouter::new(configs: &[DownloadNodeConfig], max_file_size: u64) -> Result<Self>`

For each node config:
1. Create a `tonic::transport::Channel` to `config.address`.
2. Call `NodeCapabilities::GetSupportedDomains` — on failure, log warning and treat node as having no cookies.
3. Populate `domain_cookie_map`.

### `NodeRouter::pick_node(&self, domain: Option<&str>) -> Option<Arc<NodeHandle>>`

1. If `domain` is `Some`, look up `domain_cookie_map[domain]` → collect matching nodes.
2. Filter: `node.active_downloads.load(Relaxed) < node.max_concurrent`.
3. If no matches → fall back to all nodes, apply same filter.
4. If still no matches → return `None`.
5. Pick node with lowest `active_downloads` among candidates.

### `NodeRouter::refresh_status(&self)`

Call `GetStatus` on each node. Update `active_downloads` from the response. Used by the background polling task.

### Register in DI container (`bot/src/di_container.rs`)

```rust
provide(instance(Arc::new(NodeRouter::new(&cfg.download.nodes, cfg.yt_dlp.max_file_size)?)))
```

### Background polling task (`bot/src/utils/startup.rs`)

In `on_startup`, after existing setup, spawn a background task:
```rust
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        interval.tick().await;
        router.refresh_status().await;
    }
});
```

If `download.capabilities_refresh_interval > 0`, also spawn a separate task that calls
`GetSupportedDomains` on each node and rebuilds `domain_cookie_map` every N seconds.

**Acceptance criteria:**
- `NodeRouter::new` connects to configured nodes on startup.
- `pick_node` returns `None` when all nodes are at capacity.
- `pick_node` prefers nodes with cookies for the given domain.
- Status polling updates `active_downloads` every 5 seconds.

---

## Task 9 — Bot Server: Rewrite Interactors Using gRPC

Rewrite `get_media.rs` and `download/media.rs` in the bot crate to use the gRPC client instead of yt-dlp.
**The public interfaces (struct names, `Interactor` trait impls, input/output types) must remain identical**
so that handlers in `bot/src/handlers/` do not need to change.

### `bot/src/interactors/get_media.rs`

Keep unchanged:
- All `pub struct` input/output types (`GetMediaByURLInput`, `GetMediaByURLKind`, etc.)
- All `pub struct` service types (`GetVideoByURL`, `GetAudioByURL`, `GetUncachedVideoByURL`,
  `GetShortMediaByURL`, `SearchMediaInfo`)
- `impl Interactor<...>` signatures

Change:
- `GetVideoByURL`, `GetAudioByURL`, `GetUncachedVideoByURL` now hold `Arc<NodeRouter>` instead of
  `Arc<YtDlpConfig>`, `Arc<YtPotProviderConfig>`, `Arc<Cookies>`.
- Their `execute` implementations:
  1. Call `router.pick_node(domain)` → if `None` and `overload_strategy = Reject` → return
     `GetMediaByURLErrorKind::NodeUnavailable`.
  2. Increment `node.active_downloads`.
  3. Create gRPC client: `DownloaderClient::new(node.channel.clone())` with auth token in metadata.
  4. Call `GetMediaInfo` with the request params.
  5. Decrement `node.active_downloads` after the call.
  6. Map `MediaInfoResponse` back into existing `Playlist` / `GetMediaByURLKind` types using the same
     cache-check logic as the old implementation (query Postgres before and after getting info from node).

Keep unchanged (still uses `yt_toolkit` HTTP client, no gRPC):
- `GetShortMediaByURL` — calls `yt_toolkit::get_video_info` as before.
- `SearchMediaInfo` — calls `yt_toolkit::search_video` as before.

Add to `GetMediaByURLErrorKind`:
```rust
#[error("No download node available")]
NodeUnavailable,
```

Ensure `FormatErrorToMessage` is implemented for the new variant (returns a user-visible message).

### `bot/src/interactors/download/media.rs`

Keep unchanged:
- `DownloadMediaInput`, `DownloadMediaPlaylistInput` structs and their `new_with_progress` / `new` constructors.
- `DownloadMediaErrorKind`, `DownloadMediaPlaylistErrorKind`.
- `DownloadVideo`, `DownloadAudio`, `DownloadVideoPlaylist`, `DownloadAudioPlaylist` struct names.
- `impl Interactor<...>` signatures and return types.

Change:
- Each struct now holds `Arc<NodeRouter>` instead of `Arc<YtDlpConfig>`, `Arc<YtPotProviderConfig>`,
  `Arc<TimeoutsConfig>`, `Arc<Cookies>`.
- `execute` implementations:
  1. Call `router.pick_node(url.domain())`.
  2. Increment `node.active_downloads`.
  3. Create gRPC streaming client with auth metadata.
  4. Call `DownloadMedia`.
  5. Read the stream:
     - First chunk must be `meta` → extract `ext`, `width`, `height`, `duration`.
     - `progress` chunks → forward to `progress_sender` channel (same as before).
     - `data` chunks → write to a `TempDir` temp file.
  6. After stream closes with `OK` → return `Ok(Some((MediaInFS { path, thumb_path: None, temp_dir }, format)))`.
     `thumb_path` is `None` because thumbnails are handled separately below.
  7. On stream error → map to `DownloadMediaErrorKind` and return `Err`.
  8. Decrement `node.active_downloads` in all paths.

**Thumbnail handling:** The node currently embeds thumbnails internally. Since the bot server no longer has
yt-dlp or ffmpeg, remove thumbnail embedding from the bot-side interactors. The node handles it internally
before streaming (Task 4c already does this). The `thumb_path` in `MediaInFS` returned to handlers will be
`None` — this is acceptable because Telegram does not require a thumbnail.

### Update DI registrations (`bot/src/di_container.rs`)

Replace removed `provide(...)` calls (from Task 7) with:

```rust
provide(|Inject(router): Inject<Arc<NodeRouter>>| Ok(get_media::GetVideoByURL { router: router.clone() })),
provide(|Inject(router): Inject<Arc<NodeRouter>>| Ok(get_media::GetAudioByURL { router: router.clone() })),
provide(|Inject(router): Inject<Arc<NodeRouter>>| Ok(get_media::GetUncachedVideoByURL { router: router.clone() })),
provide(|Inject(router): Inject<Arc<NodeRouter>>| Ok(download::media::DownloadVideo { router: router.clone() })),
provide(|Inject(router): Inject<Arc<NodeRouter>>| Ok(download::media::DownloadAudio { router: router.clone() })),
provide(|Inject(router): Inject<Arc<NodeRouter>>| Ok(download::media::DownloadVideoPlaylist { router: router.clone() })),
provide(|Inject(router): Inject<Arc<NodeRouter>>| Ok(download::media::DownloadAudioPlaylist { router: router.clone() })),
```

**Acceptance criteria:**
- `cargo build -p ytdl_tg_bot_bot` compiles without errors.
- No files in `bot/src/handlers/` were modified.
- No files in `bot/src/middlewares/` were modified.
- `GetShortMediaByURL` and `SearchMediaInfo` still use the HTTP yt_toolkit client, not gRPC.
- A download request flows: handler → interactor → NodeRouter → gRPC → node → stream back → Telegram upload.

---

## Task 10 — Error Mapping Reference

Use this table when mapping gRPC errors to bot-side error types. Do not invent other mappings.

| gRPC Status | Meaning | Bot server action |
|---|---|---|
| `OK` | Success | Continue normally |
| `UNAUTHENTICATED` | Wrong token sent by bot | Log `error!`, return `NodeUnavailable` |
| `RESOURCE_EXHAUSTED` | Node at capacity | Decrement counter, try next node via router |
| `UNAVAILABLE` | Node unreachable | Log `warn!`, skip node in router, try next |
| `INTERNAL` | yt-dlp failed on node | Surface to user as download error |
| `INVALID_ARGUMENT` | File too large, bad request | Surface to user as download error |
| `DEADLINE_EXCEEDED` | Timeout | Log `warn!`, return download error to user |

For `RESOURCE_EXHAUSTED` and `UNAVAILABLE`: if no other node is available after retry, return
`NodeUnavailable` which surfaces to the user as _"All download nodes are busy, try again later."_

---

## Do Not Change

The following files and modules must not be modified (unless fixing a compile error caused by
removed imports):

- `bot/src/handlers/` — all files
- `bot/src/middlewares/` — all files
- `bot/src/filters/` — all files
- `bot/src/database/` — all files
- `bot/src/entities/` — except removing `cookies.rs` module declaration
- `bot/src/handlers_utils/` — all files
- `bot/src/utils/` — except `startup.rs` for background task addition
- `bot/src/value_objects/` — all files
- Postgres schema — no migrations needed

---

## Implementation Order

Execute tasks strictly in this order. Each task must compile before starting the next.

```
Task 0  → workspace setup
Task 1  → proto definition
Task 2  → node config
Task 3  → copy services to node
Task 4  → node gRPC implementations
Task 5  → node main.rs
Task 6  → bot config changes
Task 7  → remove bot yt-dlp code
Task 8  → NodeRouter
Task 9  → rewrite bot interactors
Task 10 → verify error mapping (no code, review only)
```