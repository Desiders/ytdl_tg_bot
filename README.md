<div align="center">

<h1><code>ytdl_tg_bot</code></h1>

<h3>
A telegram bot for downloading audio and video
</h3>

</div>

Telegram: [@yv2t_bot](https://t.me/yv2t_bot)

## Features

- Auto media-type detection — send a link and the bot picks video / audio / photo for you
- Video download
- Audio download
- Photo download
- Playlist download
- Inline mode (auto / video / audio)
- Song recognition (`/shazam`) — identify a track from an audio, voice, video or video note, then download it
- Cookie-free Instagram / Facebook downloads (via [`snapsave-parser`](https://github.com/Desiders/snapsave-parser))
- Broad platform support — YouTube, TikTok, Instagram, Facebook, Twitter/X, Spotify, VK, Bluesky, Coub and more
- Language selection
- Media crop
- Skip download param
- Random media
- Reactions on supported links
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
- Database credentials in `configs/config.toml` match the `db` Secret.
- Normal downloader token from `configs/config.toml` `[download].node_token` is listed in `configs/downloader.toml` `[auth].node_tokens`.
- Cookie-manager token matches in `configs/downloader.toml` `[auth].cookie_manager_token` and `configs/cookie_assignment.toml` `[download].cookie_manager_token`.
- `configs/config.toml` `[redis].host` matches the Valkey service name (`valkey` with the bundled operator; confirm with `kubectl get svc -n "${NAMESPACE}"` after the `ValkeyCluster` reconciles).
- `configs/config.toml` `[redis].user` is `admin` and `[redis].password` matches the `valkey` Secret's `password`.
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
kubectl get valkeyclusters.valkey.io -n "${NAMESPACE}"
kubectl get secret -n "${NAMESPACE}" telegram-bot-api db db-superuser s3 valkey bot-config downloader-config cookie-assignment-config cookie-assignment-cookies
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

Use shell exports only for manual S3 checks from your machine or a debug container, for example with `mc` or `aws` CLI:

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
just docker-push-dev-migration
```

Each script builds with Docker `buildx`, pushes to the registry, uses remote cache, and defaults to `dev-<git-short-sha>`.

- Bot, downloader, and cookie-assignment scripts print the next `helm upgrade` command after a successful push.
- The migration script builds from `deployment/Dockerfile.migration.dev` and prints the next `just k8s-migration` command with `IMAGE_REPO`, `IMAGE_TAG`, and `PULL_POLICY=Always`.

Useful overrides:

- `NAMESPACE`
- `IMAGE_REPO`
- `IMAGE_TAG`
- `MIGRATION_COMMAND`
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
IMAGE_TAG="${IMAGE_TAG}" just docker-push-dev-migration
```

If only one component changed, rebuild and upgrade only that component.
