# Progress

## Current Direction

We should keep supporting the current split runtime:

- `bot` stays the Telegram-facing orchestrator
- `downloader` stays the worker that runs `yt-dlp` and streams media back
- `cookie_assignment` now owns cookie distribution and downloader cookie hygiene

The requested new task was:

- move cookie assignment out of the bot chart
- make cookie assignment independent so the bot does not know about it

## Implemented

- Updated `AGENTS.md` to match the current project structure and deployment shape.
- Added a new workspace crate: `cookie_assignment` (`ytdl_tg_cookie_assignment`).
- Moved the cookie sync loop and node cookie reconciliation logic out of the bot and into the new service.
- Added dedicated config files: `configs/cookie_assignment.toml` and `configs/cookie_assignment.example.toml`.
- Added dedicated Dockerfiles:
  - `deployment/Dockerfile.cookie_assignment`
  - `deployment/Dockerfile.cookie_assignment.dev`
- Added a dedicated dev image build script: `scripts/build-cookie-assignment-dev-image.sh`.
- Added a dedicated Helm chart: `charts/cookie-assignment`.
- Added a dedicated shared infra chart: `charts/infra`.
- Removed bot-side cookie assignment wiring from:
  - bot config
  - bot DI container
  - bot startup
  - bot deployment chart
- Removed the last bot-side `NodeCookieManager` helper methods so the bot no longer owns cookie RPC behavior.
- Removed the stale `cookies_path` entry from the downloader config template.
- Made protobuf builds self-contained by vendoring `protoc` in `proto/build.rs`.
- Fixed cookie source reconciliation:
  - if a cookie file disappears from `/app/cookies`, the controller now actively revokes it from available workers
  - removed source cookies no longer linger indefinitely on nodes
- Improved cookie placement policy for free cookies:
  - selection is no longer first-fit by address
  - the controller now prefers the eligible worker with fewer assigned cookies
  - then prefers the worker with fewer assigned domains
  - then uses address as a stable tie-break
- Cleaned up the cookie-assignment implementation by splitting the service into smaller internal helpers so the selection and reconcile logic are easier to follow.
- Moved shared cert-manager CA bootstrap resources out of `charts/bot` into `charts/infra` so app charts no longer own cluster-wide TLS bootstrap.
- Switched cookie inventory from flat filename parsing to strict directory layout: `cookies/<domain>/<cookie-id>.txt`.
- Updated cookie secret sync to flatten keys as `<domain>__<cookie-id>.txt`.
- Removed the cookie-assignment init-container reconstruction script and moved mounted entry-name decoding into the Rust loader instead.
- Added a new shared workspace crate: `downloader_client` (`ytdl_tg_downloader_client`).
- Moved downloader-node discovery, mTLS channel setup, request authentication, node selection, and node refresh logic into the shared `downloader_client` crate.
- Rewired the Telegram bot to consume the shared downloader-client crate instead of owning that implementation directly.
- Moved downloader-node failover/retry loops for `GetMediaInfo` and `DownloadMedia` into the shared `downloader_client` crate, while keeping Telegram-specific stream handling and error presentation inside the bot.
- Moved shared downloader operation error enums into `downloader_client`, so the bot now wraps client-side media-info errors and directly reuses shared download errors instead of defining duplicate base enums.
- Moved the downloader RPC adapter layer itself into `downloader_client`:
  - shared `get_media_info(...)`
  - shared `download_media(...)`
  - shared `DownloadSession` / `DownloadEvent` stream decoding
- The bot now only converts shared download events into its own upload streams and local media entities.
- Added a Telegram outbound adapter boundary for handler-side messaging:
  - introduced `MessengerPort` plus a `TelegramMessenger` implementation
  - moved handler-side text send/edit/delete and inline-answer operations behind the adapter
  - updated handler progress helpers to use the adapter instead of calling the Telegram bot directly
  - kept existing interactor orchestration in handlers for now, per the current scope
- Removed direct token-aware error formatting from handlers by adding an injected `ErrorMessageFormatter`.
- Cleaned the Telegram adapter conversion layer to use `From` impls instead of local conversion helpers for parse mode and inline-query result mapping.
- Extended the messenger port to cover bot-side media delivery operations:
  - upload video/audio
  - send cached media by remote ID
  - edit inline media by remote ID
  - send cached media groups
- Moved Telegram media send/edit/retry code out of `send_media` interactors and into the `TelegramMessenger` adapter.
- Reworked `send_media` interactors to depend on the messenger port instead of building Telegram methods directly.
- Removed the old `handlers_utils/send.rs` Telegram helper module because that transport logic now lives in the adapter.
- Switched DI and bot consumers from the concrete `TelegramMessenger` type to `Messenger` so the composition root now wires against the port instead of the adapter implementation.

## Verification

- `cargo check -p ytdl_tg_bot -p ytdl_tg_cookie_assignment -p ytdl_tg_downloader` passed.
- `cargo check -p ytdl_tg_cookie_assignment` passed after the later reconcile and balancing changes.
- `cargo check -p ytdl_tg_downloader_client -p ytdl_tg_bot` passed after extracting shared downloader-node routing/client logic.
- `cargo check -p ytdl_tg_bot` passed after introducing the handler-side Telegram adapter layer.
- `cargo fmt` could not be run in this environment because `cargo-fmt` / `rustfmt` is not installed for the available toolchains.

## Design Notes To Discuss

- Downloader cookie storage is still one file per domain path (`/tmp/cookies/<domain>.txt`) even though assignments are tracked by `cookie_id`. That works with the current “at most one cookie per domain per node” policy, but it becomes a limitation if you want multiple cookies for the same domain on one node.
- If `ListNodeCookies` fails for a worker during a cycle, current logic still treats that worker as effectively untrusted for reconciliation. You said reassigning cookies away from a down node is acceptable, so I left that behavior as-is, but it is still a deliberate tradeoff.
- The new adapter boundary is only applied at the handler/progress layer so far. `send_media` interactors and lower-level Telegram send helpers still use Telegram-specific APIs directly and can be moved behind ports later if we continue the refactor.
- The outbound adapter boundary is now used by both handlers and `send_media` interactors, but handler inputs and some port DTOs still use Telegram-shaped identifiers such as `chat_id`, `message_id`, and `inline_message_id`. If we want a stronger messenger-neutral application boundary later, those IDs should become app-level wrapper types instead of raw Telegram-shaped fields.

## Next Reasonable Steps

- Deployment notes were added to `README.md`.
- Decide whether the Telegram bot should keep the small `bot/src/services/node_router.rs` re-export wrapper for local import stability, or whether we should switch bot code to import `ytdl_tg_downloader_client` directly.
