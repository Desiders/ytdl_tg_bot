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

## Self Install

This guide assumes Kubernetes, Helm, and a working image registry. If you are deploying from this repository instead of using already-published images, build and push dev images first.

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

Final checks:

```bash
kubectl get crd certificates.cert-manager.io
kubectl get crd clusters.postgresql.cnpg.io
kubectl get crd objectstores.barmancloud.cnpg.io
kubectl get deploy -n cert-manager
kubectl get deploy -n cnpg-system
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
- Database credentials in `configs/config.toml` match the `db` Secret.
- Normal downloader token from `configs/config.toml` `[download].node_token` is listed in `configs/downloader.toml` `[auth].node_tokens`.
- Cookie-manager token matches in `configs/downloader.toml` `[auth].cookie_manager_token` and `configs/cookie_assignment.toml` `[download].cookie_manager_token`.
- Default in-cluster service URLs are correct if all charts are installed into the same namespace.

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

just k8s-migration "${NAMESPACE}"

just k8s-update-downloader-config "${NAMESPACE}"
just helm-install-downloader "${NAMESPACE}"

just k8s-update-cookie-assignment-config "${NAMESPACE}"
just k8s-sync-cookie-assignment-cookies "${NAMESPACE}"
just helm-install-cookie-assignment "${NAMESPACE}"
```

Notes:

- `infra` creates the shared internal CA issuer.
- `bot` creates PostgreSQL, RustFS, Telegram Bot API, yt-toolkit, and the bot Deployment.
- `k8s-migration` runs DB migrations after PostgreSQL exists.
- The bot pod can restart until PostgreSQL is ready and migrations have run.
- `downloader` creates the headless downloader service and worker pods.
- `cookie-assignment` distributes cookies to downloader nodes and is safe with an empty cookie inventory.

### 7. Adding Another Bot

Another bot can reuse the same downloader nodes without using the Telegram bot crate.

- Create a new bot crate, for example `bot_discord`, and depend on `downloader_client` directly.
- Create a separate chart for the new bot, usually by copying `charts/bot` only as a starting point and removing Telegram-specific resources.
- Give the new bot its own config Secret and client TLS certificate.
- Put the new bot's normal downloader token in its bot config, then add the same token to downloader config `[auth].node_tokens`.
- Do not give the new bot `cookie_manager_token`; only `cookie_assignment` should have that token.
- Keep cookie distribution in `cookie_assignment`; do not add cookie assignment logic to a bot.
- PostgreSQL, RustFS, migrations, and upload cache are optional for another bot. A simple bot can use downloader nodes without any cache.
- If another bot has a cache, design that cache for that messenger. Do not blindly reuse the Telegram `file_id` cache model.

### 8. Verify

```bash
kubectl get pods -n "${NAMESPACE}"
kubectl get certificates -n "${NAMESPACE}"
kubectl get clusters.postgresql.cnpg.io -n "${NAMESPACE}"
kubectl get backups.postgresql.cnpg.io -n "${NAMESPACE}"
kubectl get scheduledbackups.postgresql.cnpg.io -n "${NAMESPACE}"
kubectl get objectstores.barmancloud.cnpg.io -n "${NAMESPACE}"
kubectl get secret -n "${NAMESPACE}" bot-config downloader-config cookie-assignment-config cookie-assignment-cookies
```

Logs:

```bash
just k8s-logs-bot "${NAMESPACE}"
just k8s-logs-downloader "${NAMESPACE}"
just k8s-logs-cookie-assignment "${NAMESPACE}"
```

Manual backup check:

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
just k8s-migration "${NAMESPACE}"
```

Run a different migration command:

```bash
just k8s-migration "${NAMESPACE}" down
```

## Dev Images

Use this when deploying images built from your local checkout.

```bash
just docker-push-dev-bot
just docker-push-dev-downloader
just docker-push-dev-cookie-assignment
```

Each script builds with Docker `buildx`, pushes to the registry, uses remote cache, and defaults to `dev-<git-short-sha>`. The scripts print the next `helm upgrade` command after a successful push.

Useful overrides:

- `NAMESPACE`
- `IMAGE_REPO`
- `IMAGE_TAG`
- `CACHE_REF`
- `PLATFORM`
- `DOCKERFILE`
- `CONTEXT_DIR`
- `BUILDER_NAME`

Example:

```bash
export NAMESPACE=dev
export IMAGE_TAG=dev-manual-1

IMAGE_TAG="${IMAGE_TAG}" just docker-push-dev-bot
IMAGE_TAG="${IMAGE_TAG}" just docker-push-dev-downloader
IMAGE_TAG="${IMAGE_TAG}" just docker-push-dev-cookie-assignment
```

If only one component changed, rebuild and upgrade only that component.
