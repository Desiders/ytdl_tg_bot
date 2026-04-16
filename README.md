<div align="center">

<h1><code>ytdl_tg_bot</code></h1>

<h3>
A telegram bot for downloading audio and video
</h3>

</div>

Telegram: [@yv2t_bot](https://t.me/yv2t_bot)

## Features

- Video download 
- Audio download
- Playlist download
- Language selection
- Skip download param
- Random media
- Media crop
- Stats
- Exclude domains list

### Prerequisites

Cluster-side prerequisites:

- `cert-manager` must already be installed in the cluster
- CloudNativePG `1.26+` CRDs/operator must already be installed, because the bot chart creates a `postgresql.cnpg.io/v1` `Cluster` with `spec.plugins`
- For new deployments, prefer CloudNativePG `1.29+`
- Barman Cloud CNPG-I Plugin must already be installed in the same namespace as the CloudNativePG operator, because the bot chart creates a `barmancloud.cnpg.io/v1` `ObjectStore` and uses plugin-based backups

Local prerequisites:

- `kubectl`
- `helm`
- `just`

Choose a namespace first:

```bash
export NAMESPACE=bot
kubectl create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl apply -f -
```

### Required Secrets

These secrets must exist for the deployment to become healthy.

Secrets you create manually:

- `telegram-bot-api`
  - used by the `telegram-bot-api` Deployment in the bot chart
  - required keys:
    - `api_id`
    - `hash`
- `db`
  - used by CloudNativePG bootstrap and bot migration flow
  - expected keys:
    - `username`
    - `password`
- `db-superuser`
  - used by CloudNativePG as the superuser secret
  - create it with the same general username/password shape as the main DB secret
- `s3`
  - used by RustFS and the CNPG Barman Cloud Plugin `ObjectStore`
  - required keys:
    - `access-key-id`
    - `secret-access-key` (must be at least 8 characters)

Example commands:

```bash
kubectl -n "${NAMESPACE}" create secret generic telegram-bot-api \
  --from-literal=api_id='<telegram api id>' \
  --from-literal=hash='<telegram api hash>'

kubectl -n "${NAMESPACE}" create secret generic db \
  --from-literal=username='admin' \
  --from-literal=password='<db password>'

kubectl -n "${NAMESPACE}" create secret generic db-superuser \
  --from-literal=username='postgres' \
  --from-literal=password='<db superuser password>'

kubectl -n "${NAMESPACE}" create secret generic s3 \
  --from-literal=access-key-id='<s3 access key>' \
  --from-literal=secret-access-key='<at least 8 characters>'
```

The bot chart creates an internal single-node RustFS deployment by default and bootstraps the `backups` bucket through a Helm hook Job. If you disable RustFS and use an external S3-compatible service, create the bucket from `charts/bot/values.yaml` yourself before enabling backups.

Secrets generated from local config files:

- `bot-config`
  - created from `configs/config.toml`
- `downloader-config`
  - created from `configs/downloader.toml`
- `cookie-assignment-config`
  - created from `configs/cookie_assignment.toml`

Create or refresh them with:

```bash
just k8s-update-bot-config "${NAMESPACE}"
just k8s-update-downloader-config "${NAMESPACE}"
just k8s-update-cookie-assignment-config "${NAMESPACE}"
```

These helpers are safe before first install:

- they always create or update the Secret
- they only trigger a rollout if the corresponding Deployment already exists

Cookie inventory Secret:

- `cookie-assignment-cookies`
  - mounted by the cookie-assignment chart
  - source layout in the repo:

```text
cookies/<domain>/<cookie-id>.txt
```

Create or refresh it with:

```bash
just k8s-sync-cookie-assignment-cookies "${NAMESPACE}"
```

This command also creates an empty `cookie-assignment-cookies` Secret when no cookie files are present.

Only if you are not using the sync script, create the empty Secret manually:

```bash
kubectl -n "${NAMESPACE}" create secret generic cookie-assignment-cookies --dry-run=client -o yaml | kubectl apply -f -
```

TLS secrets created automatically by cert-manager:

- `bot-tls-secret`
- `downloader-tls-secret`
- `cookie-assignment-tls-secret`

Do not create those manually. They are issued from the `Certificate` resources after the `infra` chart creates `ca-issuer`.

### Config Preparation

Before deploying, review and fill:

- `configs/config.toml`
- `configs/downloader.toml`
- `configs/cookie_assignment.toml`

At minimum, make sure:

- bot token is set in `configs/config.toml`
- downloader auth token matches between bot, downloader, and cookie-assignment configs
- database credentials in `configs/config.toml` match the `db` Secret
- Telegram Bot API URL and downloader DNS assumptions match your namespace/service names

### Install Sequence

Install order matters:

1. install `infra`
2. install `bot`
3. install `downloader`
4. install `cookie-assignment`

Recommended sequence:

```bash
just helm-install-infra "${NAMESPACE}"

just k8s-update-bot-config "${NAMESPACE}"
just helm-install-bot "${NAMESPACE}"

just k8s-update-downloader-config "${NAMESPACE}"
just helm-install-downloader "${NAMESPACE}"

just k8s-update-cookie-assignment-config "${NAMESPACE}"
just k8s-sync-cookie-assignment-cookies "${NAMESPACE}"
just helm-install-cookie-assignment "${NAMESPACE}"
```

For upgrades:

```bash
just helm-upgrade-infra "${NAMESPACE}"
just helm-upgrade-bot "${NAMESPACE}"
just helm-upgrade-downloader "${NAMESPACE}"
just helm-upgrade-cookie-assignment "${NAMESPACE}"
```

If configs change, refresh the corresponding config Secret and rollout:

```bash
just k8s-update-bot-config "${NAMESPACE}"
just k8s-update-downloader-config "${NAMESPACE}"
just k8s-update-cookie-assignment-config "${NAMESPACE}"
```

If cookies change:

```bash
just k8s-sync-cookie-assignment-cookies "${NAMESPACE}"
just k8s-rollout-cookie-assignment "${NAMESPACE}"
```

### Migrations

Database migrations are applied with the dedicated helper:

```bash
just k8s-migration "${NAMESPACE}"
```

This renders the migration Job from the bot chart, applies it, waits for completion, and prints the Job logs.

To run a different migration command:

```bash
just k8s-migration "${NAMESPACE}" down
```

Run migrations:

- after the database is available
- before starting a bot version that depends on new schema changes

The migration Job uses:

- the `db` Secret for `username` and `password`
- the `postgres-rw` service from the CloudNativePG cluster created by the bot chart
- the migration image configured in `charts/bot/values.yaml`

### Validation

```bash
kubectl get pods -n "${NAMESPACE}"
kubectl get certificates -n "${NAMESPACE}"
kubectl get objectstores.barmancloud.cnpg.io -n "${NAMESPACE}"
kubectl get scheduledbackups.postgresql.cnpg.io -n "${NAMESPACE}"
kubectl get secret -n "${NAMESPACE}" bot-config downloader-config cookie-assignment-config cookie-assignment-cookies
```

Useful log commands:

```bash
just k8s-logs-bot "${NAMESPACE}"
just k8s-logs-downloader "${NAMESPACE}"
just k8s-logs-cookie-assignment "${NAMESPACE}"
```

## Dev Images

If you build and push dev images, use:

```bash
just docker-push-dev-bot
just docker-push-dev-downloader
just docker-push-dev-cookie-assignment
```

These commands use the helper scripts in `scripts/`:

- `scripts/build-bot-dev-image.sh`
- `scripts/build-downloader-dev-image.sh`
- `scripts/build-cookie-assignment-dev-image.sh`

Each script:

- builds with Docker `buildx`
- pushes directly to the registry
- uses the corresponding `*.dev` Dockerfile from `deployment/`
- uses a remote build cache
- defaults the image tag to `dev-<git-short-sha>`

Default image repositories:

- bot: `desiders/ytdl_tg_bot`
- downloader: `desiders/ytdl_tg_bot.downloader`
- cookie-assignment: `desiders/ytdl_tg_bot.cookie_assignment`

Useful environment overrides:

- `IMAGE_REPO`
- `IMAGE_TAG`
- `CACHE_REF`
- `PLATFORM`
- `DOCKERFILE`
- `CONTEXT_DIR`
- `BUILDER_NAME`

Example with an explicit tag:

```bash
IMAGE_TAG=dev-manual-1 just docker-push-dev-bot
IMAGE_TAG=dev-manual-1 just docker-push-dev-downloader
IMAGE_TAG=dev-manual-1 just docker-push-dev-cookie-assignment
```

Then roll those images into the charts:

```bash
helm upgrade bot ./charts/bot -n "${NAMESPACE}" \
  --set bot.image.repository=desiders/ytdl_tg_bot \
  --set bot.image.tag=dev-manual-1 \
  --set bot.image.pullPolicy=IfNotPresent

helm upgrade downloader ./charts/downloader -n "${NAMESPACE}" \
  --set downloader.image.repository=desiders/ytdl_tg_bot.downloader \
  --set downloader.image.tag=dev-manual-1 \
  --set downloader.image.pullPolicy=IfNotPresent

helm upgrade cookie-assignment ./charts/cookie-assignment -n "${NAMESPACE}" \
  --set cookieAssignment.image.repository=desiders/ytdl_tg_bot.cookie_assignment \
  --set cookieAssignment.image.tag=dev-manual-1 \
  --set cookieAssignment.image.pullPolicy=IfNotPresent
```

If you only changed one component, only rebuild and upgrade that chart.
