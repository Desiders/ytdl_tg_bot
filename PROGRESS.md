# Progress

## Current State

The project now has four clear runtime/deployment areas:

- `infra`: shared internal TLS bootstrap
- `bot`: Telegram-facing application plus PostgreSQL, RustFS, and database backup resources
- `downloader`: media worker nodes plus shared `yt-pot-provider`
- `cookie_assignment`: independent cookie distribution controller

The bot no longer owns cookie assignment.

## Completed

- Split cookie assignment out of the bot into its own crate, config, chart, and image flow.
- Moved shared cert-manager CA bootstrap resources into `charts/infra`.
- Switched cookie inventory to `cookies/<domain>/<cookie-id>.txt` and simplified runtime mounting by decoding flattened entry names in Rust instead of rebuilding files in an init container.
- Improved cookie reconciliation:
  - removed source cookies are actively revoked from nodes
  - free cookies are placed by least-loaded eligible worker instead of first-fit
  - empty DNS/status/list cycles keep previous assignments instead of clearing state
  - `ListNodeCookies` failures keep existing assignments for the affected worker and skip it for new assignments in that cycle
- Extracted shared downloader access into `downloader_client`:
  - node discovery
  - mTLS/auth setup
  - node selection
  - failover/retry
  - downloader RPC wrappers and stream decoding
  - cookie-assignment cookie-manager RPC wrappers
- Split downloader-node auth into normal node tokens and cookie-manager token so bots cannot call cookie mutation RPCs, and cookie assignment no longer needs any normal node token.
- Downloader now accepts a list of bot/node tokens through `[auth].node_tokens`, which allows separate tokens per bot.
- Allowed `NodeCapabilities` to accept either token so cookie assignment can check node status with the cookie-manager token only.
- Documented the intended path for adding another bot: new crate, own chart/config/TLS cert, direct `downloader_client` usage, optional messenger-specific cache, and no cookie-manager token.
- Bot startup now runs an initial downloader-node status/capability refresh before accepting updates; refresh errors are logged and ignored by the router.
- Introduced `MessengerPort` with `TelegramMessenger` as the outbound Telegram adapter.
- Moved bot-side Telegram send/edit/upload/media-group logic behind the messenger adapter.
- Reworked DI to use the current generic `Messenger` pattern instead of binding bot code directly to the concrete Telegram adapter.
- Split bot internals conceptually into:
  - `interactors` for handler-facing orchestration
  - lower-level `services` for reusable internal services and integration adapters
- Moved handler orchestration into flat top-level interactors:
  - `start`
  - `stats`
  - `config`
  - `inline_query`
  - `video`
  - `audio`
  - `chosen_inline`
- Reduced handlers to thin Telegram adapters that:
  - extract Telegram input
  - inject one top-level interactor
  - call it
- Switched PostgreSQL backups to the current CloudNativePG Barman Cloud Plugin model:
  - `ObjectStore` owns S3/RustFS backup destination configuration
  - `Cluster.spec.plugins` enables WAL archiving through `barman-cloud.cloudnative-pg.io`
  - `ScheduledBackup` uses `method: plugin`
  - RustFS stays single-node and bootstraps the `backups` bucket with a Helm hook Job

## Verification

- `cargo check -p bot -p cookie_assignment -p downloader` passed.
- `cargo check -p downloader_client -p bot` passed.
- `cargo check -p bot` passed after the latest handler-to-interactor refactor.
- `cargo check -p downloader_client -p downloader -p cookie_assignment -p bot` passed after downloader auth split and cookie-assignment client reuse.
- `cargo check -p bot -p cookie_assignment -p downloader -p downloader_client -p proto` passed after shortening local crate package names.
- `cargo fmt` could not be run here because `rustfmt` is not available in the current toolchain environment.

## Open Notes

- Downloader cookie storage is still one file per domain in `/tmp/cookies/<domain>.txt`. That is acceptable with the current rule “at most one cookie per domain per node”, but it is still a structural limitation.
- The outbound Telegram boundary is now centralized, but interactor input DTOs still carry Telegram-shaped identifiers like `chat_id`, `message_id`, and `inline_message_id`.
- Inbound transport extraction is still handler-side. Handlers are thin now, but Telegram update mapping has not been moved into a stronger inbound adapter layer.
