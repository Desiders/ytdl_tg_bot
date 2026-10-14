set dotenv-load

lint:
    cargo clippy --all --all-features -- -W clippy::pedantic

fmt:
    cargo +nightly fmt --all

@docker-build VERSION="latest":
    docker build -f ./deployment/Dockerfile.bot -t desiders/ytdl_tg_bot:{{VERSION}} .

@docker-build-cookie-assignment VERSION="latest":
    docker build -f ./deployment/Dockerfile.cookie_assignment -t desiders/ytdl_tg_bot.cookie_assignment:{{VERSION}} .

@docker-build-downloader VERSION="latest":
    docker build -f ./deployment/Dockerfile.downloader -t desiders/ytdl_tg_bot.downloader:{{VERSION}} .

@docker-build-migration VERSION="latest":
    docker build -f ./deployment/Dockerfile.migration -t desiders/ytdl_tg_bot.migration:{{VERSION}} .

docker-compose-tls:
    #!/usr/bin/env bash
    set -euo pipefail
    umask 077
    openssl version >/dev/null
    mkdir -p tls
    cd tls
    ensure_key() {
        if ! openssl pkey -in "$1.key" -check -noout </dev/null >/dev/null 2>&1; then
            openssl genrsa -out "$1.key" 2048
        fi
    }
    pair_matches() {
        [[ -s "$1.crt" ]] && [[ "$(openssl x509 -in "$1.crt" -pubkey -noout 2>/dev/null)" == "$(openssl pkey -in "$1.key" -pubout)" ]]
    }
    ensure_key ca
    if ! pair_matches ca || ! openssl verify -CAfile ca.crt ca.crt >/dev/null 2>&1 || \
        [[ "$(openssl x509 -in ca.crt -noout -ext basicConstraints)" != *CA:TRUE* ]]; then
        openssl req -new -x509 -key ca.key -days 3650 -sha256 -subj "/CN=ytdl-tg-bot-dev-ca" \
            -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" -out ca.crt
    fi
    for identity in bot node cookie-assignment; do
        ensure_key "$identity"
        purpose=sslclient; usage=clientAuth; san=; host_check=()
        if [[ "$identity" == node ]]; then
            purpose=sslserver; usage=serverAuth; san="subjectAltName=DNS:downloader"; host_check=(-verify_hostname downloader)
        fi
        if pair_matches "$identity" && openssl verify -CAfile ca.crt -purpose "$purpose" "${host_check[@]}" "$identity.crt" >/dev/null 2>&1; then
            continue
        fi
        openssl req -new -key "$identity.key" -subj "/CN=$identity" -out "$identity.csr"
        openssl x509 -req -in "$identity.csr" -CA ca.crt -CAkey ca.key -set_serial "0x$(openssl rand -hex 16)" \
            -days 365 -sha256 -extfile <(printf '%s\n' "basicConstraints=critical,CA:FALSE" \
                "keyUsage=critical,digitalSignature,keyEncipherment" "extendedKeyUsage=$usage" "$san") -out "$identity.crt"
        rm "$identity.csr"
    done
    chmod 600 *.key
    echo "Development TLS certificates are ready"

@docker-compose-up: docker-compose-tls
    docker compose up --build

@docker-compose-down:
    docker compose down

@docker-compose-logs:
    docker compose logs -f

@docker-compose-migration COMMAND="up":
    docker compose run --build --rm migrate {{COMMAND}}

docker-push USER VERSION="latest":
    @just docker-build {{VERSION}}
    docker push {{USER}}/ytdl_tg_bot:{{VERSION}}

docker-push-cookie-assignment USER VERSION="latest":
    @just docker-build-cookie-assignment {{VERSION}}
    docker push {{USER}}/ytdl_tg_bot.cookie_assignment:{{VERSION}}

docker-push-downloader USER VERSION="latest":
    @just docker-build-downloader {{VERSION}}
    docker push {{USER}}/ytdl_tg_bot.downloader:{{VERSION}}

docker-push-migration USER VERSION="latest":
    @just docker-build-migration {{VERSION}}
    docker push {{USER}}/ytdl_tg_bot.migration:{{VERSION}}

k3s-stop:
    sudo systemctl stop k3s

k3s-start:
    sudo systemctl start k3s

k3s-restart:
    sudo systemctl restart k3s

k3s-killall:
    sudo /usr/local/bin/k3s-killall.sh

helm-install-bot NAMESPACE:
    helm install bot ./charts/bot -n {{NAMESPACE}} --create-namespace

helm-install-infra NAMESPACE:
    helm install infra ./charts/infra -n {{NAMESPACE}} --create-namespace

helm-upgrade-bot NAMESPACE:
    helm upgrade bot ./charts/bot -n {{NAMESPACE}} --reset-values

helm-upgrade-infra NAMESPACE:
    helm upgrade infra ./charts/infra -n {{NAMESPACE}} --reset-values

helm-install-downloader NAMESPACE:
    helm install downloader ./charts/downloader -n {{NAMESPACE}} --create-namespace

helm-install-cookie-assignment NAMESPACE:
    helm install cookie-assignment ./charts/cookie-assignment -n {{NAMESPACE}} --create-namespace

helm-upgrade-downloader NAMESPACE:
    helm upgrade downloader ./charts/downloader -n {{NAMESPACE}} --reset-values

helm-upgrade-cookie-assignment NAMESPACE:
    helm upgrade cookie-assignment ./charts/cookie-assignment -n {{NAMESPACE}} --reset-values

scale-downloader NAMESPACE REPLICAS="1":
    helm upgrade downloader ./charts/downloader -n {{NAMESPACE}} --set downloader.replicas={{REPLICAS}}

k8s-rollout-bot NAMESPACE:
    kubectl rollout restart deployment/bot -n {{NAMESPACE}}

k8s-rollout-downloader NAMESPACE:
    kubectl rollout restart deployment/downloader -n {{NAMESPACE}}

k8s-rollout-cookie-assignment NAMESPACE:
    kubectl rollout restart deployment/cookie-assignment -n {{NAMESPACE}}

k8s-update-bot-config NAMESPACE:
    kubectl create secret generic bot-config --from-file=config.toml=./configs/config.toml --dry-run=client -o yaml | kubectl apply -n {{NAMESPACE}} -f -
    if kubectl get deployment/bot -n {{NAMESPACE}} >/dev/null 2>&1; then just k8s-rollout-bot {{NAMESPACE}}; fi

k8s-sync-cookie-assignment-cookies NAMESPACE SECRET_NAME="cookie-assignment-cookies" SOURCE_DIR="cookies":
    NAMESPACE={{NAMESPACE}} SECRET_NAME={{SECRET_NAME}} ./scripts/sync-cookies-secret.sh {{SOURCE_DIR}}

k8s-update-downloader-config NAMESPACE:
    kubectl create secret generic downloader-config --from-file=downloader.toml=./configs/downloader.toml --dry-run=client -o yaml | kubectl apply -n {{NAMESPACE}} -f -
    if kubectl get deployment/downloader -n {{NAMESPACE}} >/dev/null 2>&1; then just k8s-rollout-downloader {{NAMESPACE}}; fi

k8s-update-cookie-assignment-config NAMESPACE:
    kubectl create secret generic cookie-assignment-config --from-file=cookie_assignment.toml=./configs/cookie_assignment.toml --dry-run=client -o yaml | kubectl apply -n {{NAMESPACE}} -f -
    if kubectl get deployment/cookie-assignment -n {{NAMESPACE}} >/dev/null 2>&1; then just k8s-rollout-cookie-assignment {{NAMESPACE}}; fi

k8s-port-forward-db NAMESPACE LOCAL_PORT="5432":
    kubectl -n {{NAMESPACE}} port-forward svc/postgres-rw {{LOCAL_PORT}}:5432

k8s-migration NAMESPACE COMMAND="up":
    if [ -n "${IMAGE_REPO:-}" ] && [ -z "${IMAGE_TAG:-}" ]; then echo "IMAGE_TAG is required when IMAGE_REPO is set" >&2; exit 1; fi
    if [ -z "${IMAGE_REPO:-}" ] && [ -n "${IMAGE_TAG:-}" ]; then echo "IMAGE_REPO is required when IMAGE_TAG is set" >&2; exit 1; fi
    run_id=$(date +%s); \
    job_name=bot-migration-$run_id; \
    if [ -n "${IMAGE_REPO:-}" ]; then echo "Migration image: ${IMAGE_REPO}:${IMAGE_TAG}"; fi; \
    helm template bot ./charts/bot -n {{NAMESPACE}} \
      --show-only templates/migration-job.yaml \
      --set migration.enabled=true \
      --set migration.runId=$run_id \
      --set-string migration.command='{{COMMAND}}' \
      $(if [ -n "${IMAGE_REPO:-}" ]; then printf '%s' "--set migration.image.repository=${IMAGE_REPO}"; fi) \
      $(if [ -n "${IMAGE_TAG:-}" ]; then printf '%s' "--set migration.image.tag=${IMAGE_TAG}"; fi) \
      $(if [ -n "${IMAGE_REPO:-}" ]; then printf '%s' "--set migration.image.pullPolicy=${PULL_POLICY:-Always}"; fi) \
      | kubectl apply -n {{NAMESPACE}} -f -; \
    if ! kubectl wait -n {{NAMESPACE}} --for=condition=complete --timeout=10m job/$job_name; then \
      kubectl logs -n {{NAMESPACE}} job/$job_name --all-containers=true || true; \
      exit 1; \
    fi; \
    kubectl logs -n {{NAMESPACE}} job/$job_name --all-containers=true

k8s-logs-bot NAMESPACE:
    kubectl logs -l app=bot -n {{NAMESPACE}} -f

k8s-logs-downloader NAMESPACE:
    kubectl logs -l app=downloader -n {{NAMESPACE}} -f

k8s-logs-cookie-assignment NAMESPACE:
    kubectl logs -l app=cookie-assignment -n {{NAMESPACE}} -f

k8s-logs-db NAMESPACE:
    kubectl logs -l cnpg.io/cluster=postgres -n {{NAMESPACE}} -f

k8s-logs-telegram-api NAMESPACE:
    kubectl logs -l app=telegram-bot-api -n {{NAMESPACE}}

k8s-logs-yt-toolkit NAMESPACE:
    kubectl logs -l app=yt-toolkit-api -n {{NAMESPACE}}

k8s-logs-pot-provider NAMESPACE:
    kubectl logs -l app=yt-pot-provider -n {{NAMESPACE}}

k8s-logs-valkey NAMESPACE:
    kubectl logs -l valkey.io/cluster=valkey -n {{NAMESPACE}} -f

help:
    just -l
