# Progress

## Current State

The project now has four clear runtime/deployment areas:

- `infra`: shared internal TLS bootstrap
- `bot`: Telegram-facing application
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
- Extracted shared downloader access into `downloader_client`:
  - node discovery
  - mTLS/auth setup
  - node selection
  - failover/retry
  - downloader RPC wrappers and stream decoding
- Introduced `MessengerPort` with `TelegramMessenger` as the outbound Telegram adapter.
- Moved bot-side Telegram send/edit/upload/media-group logic behind the messenger adapter.
- Reworked DI to use the current generic `Messenger` pattern instead of binding bot code directly to the concrete Telegram adapter.
- Split bot internals conceptually into:
  - `use_cases` for handler-facing orchestration
  - lower-level `services` for reusable internal services and integration adapters
- Moved handler orchestration into flat use cases:
  - `start`
  - `stats`
  - `config`
  - `inline_query`
  - `video`
  - `audio`
  - `chosen_inline`
- Reduced handlers to thin Telegram adapters that:
  - extract Telegram input
  - inject one use case
  - call it

## Verification

- `cargo check -p ytdl_tg_bot -p ytdl_tg_cookie_assignment -p ytdl_tg_downloader` passed.
- `cargo check -p ytdl_tg_downloader_client -p ytdl_tg_bot` passed.
- `cargo check -p ytdl_tg_bot` passed after the latest handler-to-interactor refactor.
- `cargo fmt` could not be run here because `rustfmt` is not available in the current toolchain environment.

## Open Notes

- Downloader cookie storage is still one file per domain in `/tmp/cookies/<domain>.txt`. That is acceptable with the current rule “at most one cookie per domain per node”, but it is still a structural limitation.
- If `ListNodeCookies` fails for a worker during a reconcile cycle, current cookie-assignment behavior still treats that worker as untrusted for reconciliation. This was kept intentionally.
- The outbound Telegram boundary is now centralized, but interactor input DTOs still carry Telegram-shaped identifiers like `chat_id`, `message_id`, and `inline_message_id`.
- Inbound transport extraction is still handler-side. Handlers are thin now, but Telegram update mapping has not been moved into a stronger inbound adapter layer.
