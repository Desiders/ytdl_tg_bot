<div align="center">

<h1><code>ytdl_tg_bot</code></h1>

<h3>
A telegram bot for downloading audio and video
</h3>

</div>

Telegram: [@yv2t_bot](https://t.me/yv2t_bot)

## Contents

- [Features](#features)
- [Commands](#commands)
- [Local Development](#local-development)
- [Checks](#checks)
- [Kubernetes Deployment](#kubernetes-deployment)
- [Operations](#operations)
- [Backups and Recovery](#backups-and-recovery)

## Features

- Auto media-type detection — send a link and the bot picks video / audio / photo for you
- Video download
- Audio download
- Photo download
- Playlist download
- Inline mode (auto / video / audio)
- Song recognition (`/shazam`) — identify a track from an audio, voice, video or video note, then download it
- Cookie-free Instagram / Facebook downloads (via [`snapsave-parser`](https://github.com/Desiders/snapsave-parser))
- Spotify tracks, albums and playlists — resolved to DRM-free sources via [`spotdl`](https://github.com/spotDL/spotify-downloader)
- Broad platform support — YouTube, TikTok, Instagram, Facebook, Twitter/X, Spotify, VK, Bluesky, Coub and more
- Durable download queue — jobs survive bot restarts (Valkey streams)
- Cached re-sends — already downloaded media is sent by Telegram `file_id` without touching a downloader node
- Language selection (English, Russian, Ukrainian)
- Media crop
- Skip download param
- Random media
- Reactions on supported links
- Stats
- Exclude domains list

## Commands

| Command | Description |
| --- | --- |
| `/vd`, `/video` | Download video with progress messages |
| `/ad`, `/audio` | Download audio |
| `/pd`, `/photo` | Download photo |
| `/rv`, `/ra` | Random video or audio from the configured domains |
| `/shazam`, `/sh` | Recognize a song from a replied voice, audio, video or video note, then download it |
| `/add_ed`, `/rm_ed` | Exclude or re-include a domain for the current chat |
| `/change_link_visibility` | Show or hide the source link in captions (private chats) |
| `/stats` | Cache, node, queue and chat statistics |
| `/lang` | Switch interface language |
| `/start`, `/help` | In a private chat, a button menu for settings, language, excluded domains, help and stats; in groups, help text with all arguments |

A bare link in a private chat is downloaded automatically with media-type detection; in groups the same happens silently. Append `yv2t_bot=false` to a link to make the bot ignore it.

Arguments go in square brackets after the command, for example `/vd [lang=ru,items=1:3:1,crop=00:01:30-,overwrite=true]`. Inline mode (`@bot <url>` or `@bot <title>`) supports `lang` and `overwrite`.

## Local Development

You need Docker with Compose v2, `just`, and OpenSSL 3. Run commands from the repository root.

### Prepare

Copy the examples; existing files are preserved:

```bash
cp -n .env.example .env
cp -n configs/config.example.toml configs/config.toml
cp -n configs/downloader.example.toml configs/downloader.toml
cp -n configs/cookie_assignment.example.toml configs/cookie_assignment.toml
```

In `configs/config.toml`, set `[bot].token` from [BotFather](https://t.me/BotFather) and
`[chat].receiver_chat_id` to a chat accessible to the bot. For a private receiver chat, send
`/start` to the bot first. Update these existing sections:

```toml
[database]
host = "postgres"
user = "bot"
password = "bot"
database = "api"

[redis]
host = "valkey"
user = "admin"
password = "valkey"

[telegram_bot_api]
url = "http://telegram-bot-api:8081"
file_server_url = "http://telegram-bot-api-files:8080"
work_dir = "/var/lib/telegram-bot-api"
```

Keep the other template values. In `.env`, set `TELEGRAM_API_ID` and `TELEGRAM_API_HASH` from
[my.telegram.org](https://my.telegram.org).
If the database ports are occupied, change `POSTGRES_PORT` or `VALKEY_PORT` in `.env`.

In `configs/downloader.toml`, enable `[spotdl].enabled` for Spotify,
`[snapsave].enabled` for cookie-free Instagram/Facebook, and `[songrec].enabled` for `/shazam`
if you need these features. They are disabled in the template.

Stop other instances using the same bot token. If this bot previously used the cloud API,
call [`logOut`](https://core.telegram.org/bots/api#logout) before starting the local server.

Optional cookies go in `cookies/<domain>/<cookie-id>.txt`.

### Run

```bash
just docker-compose-up
```

The command builds and starts the stack and shows logs from every service. Migrations run
automatically; the migration container exiting with status 0 is expected. Press Ctrl+C to stop
all services and keep their data. Run the same command again after code or config changes.

### Regenerate Database Models

SeaORM generates models from the PostgreSQL schema. New migrations in `migration/src/`
must be registered in `migration/src/lib.rs`; the
[migration generator](migration/README.md#migrator-cli) does this automatically.
Apply them before regenerating models.

Install the CLI version used in `Cargo.lock` once (requires Rust on the host):

```bash
cargo install sea-orm-cli --version 2.0.4 --locked
```

With the Compose stack running, open another terminal in the repository root. Replace
`<db-user>`, `<db-password>`, and `<db-name>` with `[database].user`, `[database].password`,
and `[database].database` from `configs/config.toml`:

```bash
just docker-compose-migration up

DATABASE_URL="postgres://<db-user>:<db-password>@$(docker compose port postgres 5432)/<db-name>" \
    sea-orm-cli generate entity \
        -o bot/src/database/models \
        --date-time-crate time \
        --with-prelude none \
        --banner-version patch \
        --entity-format dense && \
    rm -f bot/src/database/models/mod.rs
```

The database address uses the published port from Compose, including any `POSTGRES_PORT`
override. Modules are declared in `bot/src/database/models.rs`, so remove the generated
`mod.rs` and add a `pub mod` declaration there for each new table.

## Checks

Run checks from the host:

```bash
cargo check -p bot            # or downloader, downloader_client, cookie_assignment
just lint                     # clippy::pedantic across the workspace
just fmt                      # nightly rustfmt
cargo test -p downloader_client
cargo test -p bot             # database tests start PostgreSQL via testcontainers and need Docker
```

## Kubernetes Deployment

This guide assumes Kubernetes, Helm, and a working image registry. If you are deploying from this repository instead of using already-published images, build and push the images first.

### 1. Prerequisites

Required local tools:

- `kubectl`
- `helm`
- `just`

Check local tools:

```bash
kubectl version --client
helm version
just --version
```

Cluster components:

- `cert-manager`
- CloudNativePG `1.26+`; use `1.29+` for new installs
- Barman Cloud CNPG-I Plugin installed in the same namespace as the CloudNativePG operator
- Valkey operator (`valkey-operator`) for the durable download queue

Install `cert-manager`:

```bash
helm install cert-manager oci://quay.io/jetstack/charts/cert-manager \
  --version v1.20.2 \
  --namespace cert-manager \
  --create-namespace \
  --set crds.enabled=true \
  --wait

kubectl rollout status deployment/cert-manager -n cert-manager
kubectl rollout status deployment/cert-manager-webhook -n cert-manager
kubectl rollout status deployment/cert-manager-cainjector -n cert-manager
```

Install CloudNativePG:

```bash
kubectl apply --server-side -f \
  https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-1.29/releases/cnpg-1.29.0.yaml

kubectl rollout status deployment/cnpg-controller-manager -n cnpg-system
```

Install the Barman Cloud CNPG-I Plugin:

```bash
kubectl get deployment -n cnpg-system cnpg-controller-manager \
  -o jsonpath="{.spec.template.spec.containers[*].image}{'\n'}"

kubectl apply -f \
  https://github.com/cloudnative-pg/plugin-barman-cloud/releases/download/v0.12.0/manifest.yaml

kubectl rollout status deployment/barman-cloud -n cnpg-system
```

Install the Valkey operator (backs the durable download queue):

```bash
helm repo add valkey https://valkey.io/valkey-helm/
helm repo update

helm install valkey-operator valkey/valkey-operator \
  -n valkey-operator-system --create-namespace

kubectl get pods -n valkey-operator-system -l app.kubernetes.io/instance=valkey-operator
```

Final checks:

```bash
kubectl get crd certificates.cert-manager.io
kubectl get crd clusters.postgresql.cnpg.io
kubectl get crd objectstores.barmancloud.cnpg.io
kubectl get crd valkeyclusters.valkey.io
kubectl get deploy -n cert-manager
kubectl get deploy -n cnpg-system
kubectl get deploy -n valkey-operator-system
```

### 2. Namespace

```bash
export NAMESPACE=dev
kubectl create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl apply -f -
```

Use `NAMESPACE=prod` for production.

### 3. Required Secrets

Create these once. These commands are safe to re-run because they render YAML client-side and apply it.

```bash
kubectl -n "${NAMESPACE}" create secret generic telegram-bot-api \
  --from-literal=api_id='<telegram api id>' \
  --from-literal=hash='<telegram api hash>' \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl -n "${NAMESPACE}" create secret generic db \
  --from-literal=username='admin' \
  --from-literal=password='<db password>' \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl -n "${NAMESPACE}" create secret generic db-superuser \
  --from-literal=username='postgres' \
  --from-literal=password='<db superuser password>' \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl -n "${NAMESPACE}" create secret generic s3 \
  --from-literal=access-key-id='rustfsadmin' \
  --from-literal=secret-access-key='<at least 8 characters>' \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl -n "${NAMESPACE}" create secret generic valkey \
  --from-literal=password='<valkey password>' \
  --dry-run=client -o yaml | kubectl apply -f -
```

The `s3` Secret is used by the internal RustFS backup store and CNPG backup plugin. The chart creates a single-node RustFS instance and bootstraps the `backups` bucket automatically.

### 4. Config Files

Create local config files if they do not exist:

```bash
cp -n configs/config.example.toml configs/config.toml
cp -n configs/downloader.example.toml configs/downloader.toml
cp -n configs/cookie_assignment.example.toml configs/cookie_assignment.toml
```

Edit these files before installing:

- `configs/config.toml`
- `configs/downloader.toml`
- `configs/cookie_assignment.toml`

Required config checks:

- `configs/config.toml` has the real Telegram bot token.
- `[chat].receiver_chat_id` is a chat accessible to the bot. For a private receiver chat, send `/start` to the bot first.
- Database credentials in `configs/config.toml` match the `db` Secret.
- `[database].host` is `postgres-rw`, and `[database].database` matches `postgres.database` in `charts/bot/values.yaml` (`api` by default).
- Normal downloader token from `configs/config.toml` `[download].node_token` is listed in `configs/downloader.toml` `[auth].node_tokens`.
- Cookie-manager token matches in `configs/downloader.toml` `[auth].cookie_manager_token` and `configs/cookie_assignment.toml` `[download].cookie_manager_token`.
- `configs/config.toml` `[redis].host` matches the Valkey service name (`valkey` with the bundled operator; confirm with `kubectl get svc -n "${NAMESPACE}"` after the `ValkeyCluster` reconciles).
- `configs/config.toml` `[redis].user` is `admin` and `[redis].password` matches the `valkey` Secret's `password`.
- `configs/config.toml` `[telegram_bot_api]` keeps `file_server_url` and `work_dir` when using the bundled local Bot API server; remove both only when pointing `url` at `https://api.telegram.org`.
- For the bundled server, `[telegram_bot_api].url` is `http://telegram-bot-api:8081` and `file_server_url` is `http://telegram-bot-api:8080`.
- Default in-cluster service URLs are correct if all charts are installed into the same namespace.

Optional downloader features are off by default in `configs/downloader.toml`; enable the ones you need:

- `[spotdl].enabled` — Spotify links.
- `[snapsave].enabled` — cookie-free Instagram / Facebook (optionally through `[snapsave].proxy`).
- `[songrec].enabled` — `/shazam`.
- `[[user_agents]]` — per-domain `User-Agent` overrides for yt-dlp.
- `[[replace_domains.<video|audio|photo>]]` — domain rewrites used when the node has no cookie for the site.

### 5. Optional Cookies

Cookie files are optional. Put them here if you have them:

```text
cookies/<domain>/<cookie-id>.txt
```

The sync helper also creates an empty cookie Secret when no cookie files exist.

### 6. Install

Install order matters:

```bash
just helm-install-infra "${NAMESPACE}"

just k8s-update-bot-config "${NAMESPACE}"
just helm-install-bot "${NAMESPACE}"

kubectl wait -n "${NAMESPACE}" --for=condition=Ready --timeout=5m clusters.postgresql.cnpg.io/postgres
just k8s-migration "${NAMESPACE}" up

just k8s-update-downloader-config "${NAMESPACE}"
just helm-install-downloader "${NAMESPACE}"

just k8s-update-cookie-assignment-config "${NAMESPACE}"
just k8s-sync-cookie-assignment-cookies "${NAMESPACE}"
just helm-install-cookie-assignment "${NAMESPACE}"
```

Notes:

- `infra` creates the shared internal CA issuer.
- `bot` creates PostgreSQL, Valkey, RustFS, Telegram Bot API with its file server, yt-toolkit, and the bot Deployment.
- `k8s-migration` runs DB migrations after PostgreSQL exists.
- The bot pod can restart until PostgreSQL is ready and migrations have run.
- `downloader` creates the headless downloader service and worker pods.
- `cookie-assignment` distributes cookies to downloader nodes and is safe with an empty cookie inventory.

### 7. Verify

```bash
kubectl get pods -n "${NAMESPACE}"
kubectl get certificates -n "${NAMESPACE}"
kubectl get clusters.postgresql.cnpg.io -n "${NAMESPACE}"
kubectl get backups.postgresql.cnpg.io -n "${NAMESPACE}"
kubectl get scheduledbackups.postgresql.cnpg.io -n "${NAMESPACE}"
kubectl get objectstores.barmancloud.cnpg.io -n "${NAMESPACE}"
kubectl get valkeyclusters.valkey.io -n "${NAMESPACE}"
kubectl get secret -n "${NAMESPACE}" telegram-bot-api db db-superuser s3 valkey bot-config downloader-config cookie-assignment-config cookie-assignment-cookies
```

Logs:

```bash
just k8s-logs-bot "${NAMESPACE}"
just k8s-logs-downloader "${NAMESPACE}"
just k8s-logs-cookie-assignment "${NAMESPACE}"
just k8s-logs-db "${NAMESPACE}"
just k8s-logs-valkey "${NAMESPACE}"
just k8s-logs-telegram-api "${NAMESPACE}"
just k8s-logs-yt-toolkit "${NAMESPACE}"
just k8s-logs-pot-provider "${NAMESPACE}"
```

## Operations

Upgrade charts:

```bash
just helm-upgrade-infra "${NAMESPACE}"
just helm-upgrade-bot "${NAMESPACE}"
just helm-upgrade-downloader "${NAMESPACE}"
just helm-upgrade-cookie-assignment "${NAMESPACE}"
```

Refresh configs:

```bash
just k8s-update-bot-config "${NAMESPACE}"
just k8s-update-downloader-config "${NAMESPACE}"
just k8s-update-cookie-assignment-config "${NAMESPACE}"
```

Refresh cookies:

```bash
just k8s-sync-cookie-assignment-cookies "${NAMESPACE}"
just k8s-rollout-cookie-assignment "${NAMESPACE}"
```

Scale downloader nodes:

```bash
just scale-downloader "${NAMESPACE}" 3
```

Run migrations:

```bash
just k8s-migration "${NAMESPACE}" up
```

Run a different migration command:

```bash
just k8s-migration "${NAMESPACE}" down
```

Reach the database from your machine:

```bash
just k8s-port-forward-db "${NAMESPACE}"
```

To regenerate models after applying migrations, use the SeaORM CLI command in
[Regenerate Database Models](#regenerate-database-models). Replace its `DATABASE_URL`
assignment with `postgres://<db-user>:<db-password>@127.0.0.1:5432/<db-name>`, using the
credentials from the cluster's `db` Secret and `[database].database` from `configs/config.toml`.

## Backups and Recovery

### Manual Backup Check

```bash
kubectl -n "${NAMESPACE}" delete backup postgres-manual-backup --ignore-not-found

cat > /tmp/postgres-manual-backup.yaml <<'EOF'
apiVersion: postgresql.cnpg.io/v1
kind: Backup
metadata:
  name: postgres-manual-backup
spec:
  cluster:
    name: postgres
  method: plugin
  pluginConfiguration:
    name: barman-cloud.cloudnative-pg.io
EOF

kubectl -n "${NAMESPACE}" create -f /tmp/postgres-manual-backup.yaml
kubectl -n "${NAMESPACE}" get backup postgres-manual-backup -w
```

### How Barman backup config is applied

This project does not require you to manually set Barman environment variables inside the PostgreSQL pod.

The bot chart applies backup configuration in three steps:

1. The `s3` Secret provides credentials:
   - `access-key-id`
   - `secret-access-key`
2. `charts/bot/templates/postgres-backup-object-store.yaml` creates a `barmancloud.cnpg.io/v1` `ObjectStore` using:
   - `s3.endpointURL`
   - `s3.destinationPath`
   - credentials from the `s3` Secret
3. `charts/bot/templates/postgres-cluster.yaml` attaches that object store to the `postgres` cluster through `spec.plugins`, and `charts/bot/templates/postgres-scheduled-backup.yaml` creates the scheduled backup.

Current defaults from `charts/bot/values.yaml`:

- object store name: `postgres-backup-store`
- destination path: `s3://backups/`
- endpoint URL: `http://s3:9000`
- backup schedule: `0 0 3 * * *`

To inspect the applied backup config:

```bash
kubectl -n "${NAMESPACE}" get objectstores.barmancloud.cnpg.io postgres-backup-store -o yaml
kubectl -n "${NAMESPACE}" get cluster postgres -o yaml
kubectl -n "${NAMESPACE}" get scheduledbackup postgres-backup -o yaml
kubectl -n "${NAMESPACE}" get secret s3 -o yaml
```

If you change backup credentials or endpoint settings:

1. Update the `s3` Secret.
2. Update Helm values if `endpointURL`, `destinationPath`, or object-store naming changed.
3. Run:

```bash
just helm-upgrade-bot "${NAMESPACE}"
```

Use these shell exports for manual S3 checks with `mc` or `aws` CLI in a debug pod in the application's namespace:

```bash
export AWS_ACCESS_KEY_ID='<value from s3 secret access-key-id>'
export AWS_SECRET_ACCESS_KEY='<value from s3 secret secret-access-key>'
export AWS_ENDPOINT_URL='http://s3:9000'
```

Those exports are not consumed by the PostgreSQL pod directly in this setup; they are only for your manual tooling.

### How to use the backups

In this setup, backups are used for recovery, not for direct browsing from PostgreSQL.

The normal restore flow is:

1. Keep the original `postgres` cluster untouched.
2. Create a new cluster, for example `postgres-restore`.
3. Bootstrap that new cluster from the backup object store.
4. Connect to the restored cluster and inspect or export data from it.

Important:

- Recovery is not in-place. Do not try to “restore over” the running `postgres` cluster.
- The backup object store keeps base backups and WAL archives.
- The restored cluster does not need to archive WALs back to the bucket unless you explicitly configure that.

Example restore manifest for this project:

```bash
cat > /tmp/postgres-restore.yaml <<'EOF'
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: postgres-restore
spec:
  instances: 1
  storage:
    size: 3Gi
  superuserSecret:
    name: db-superuser
  bootstrap:
    recovery:
      source: origin
  externalClusters:
    - name: origin
      plugin:
        name: barman-cloud.cloudnative-pg.io
        parameters:
          barmanObjectName: postgres-backup-store
          serverName: postgres
EOF

kubectl apply -n "${NAMESPACE}" -f /tmp/postgres-restore.yaml
kubectl get clusters.postgresql.cnpg.io -n "${NAMESPACE}"
kubectl get pods -n "${NAMESPACE}" -l cnpg.io/cluster=postgres-restore -w
```

When the restored cluster is ready, connect to it:

```bash
export RESTORE_POD="$(kubectl get pod -n "${NAMESPACE}" \
  -l cnpg.io/cluster=postgres-restore,role=primary \
  -o jsonpath='{.items[0].metadata.name}')"

kubectl exec -it -n "${NAMESPACE}" "${RESTORE_POD}" -- psql -U postgres -d api
```

Then you can:

- inspect tables
- run queries
- export data with `pg_dump`
- compare restored data with the live cluster

Example `pg_dump` inside the restored pod:

```bash
kubectl exec -it -n "${NAMESPACE}" "${RESTORE_POD}" -- \
  pg_dump -U postgres -d api > /tmp/postgres-restore.sql
```

If you need point-in-time recovery instead of the latest state, add a recovery target:

```yaml
bootstrap:
  recovery:
    source: origin
    recoveryTarget:
      targetTime: "2026-04-16T21:55:00Z"
```

If you no longer need the restored cluster:

```bash
kubectl delete cluster postgres-restore -n "${NAMESPACE}"
```

### Restore from an external SQL dump

This is a different flow from Barman recovery.

Use this when you already have a dump file outside CNPG/Barman, for example from `pg_dump`.

Use a temporary cluster if possible. Do not import unknown dumps directly into the live `postgres` cluster.

Plain SQL dump:

```bash
kubectl exec -i -n "${NAMESPACE}" -c postgres "${RESTORE_POD}" -- \
  psql -U postgres -d api < /path/to/dump.sql
```

Gzipped SQL dump:

```bash
gunzip -c /path/to/dump.sql.gz | \
kubectl exec -i -n "${NAMESPACE}" -c postgres "${RESTORE_POD}" -- \
  psql -U postgres -d api
```

Custom-format dump created with `pg_dump -Fc`:

```bash
cat /path/to/dump.dump | \
kubectl exec -i -n "${NAMESPACE}" -c postgres "${RESTORE_POD}" -- \
  pg_restore -U postgres -d api --clean --if-exists
```

Notes:

- plain `.sql` dumps are restored with `psql`
- custom-format dumps are restored with `pg_restore`
