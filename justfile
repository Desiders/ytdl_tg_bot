set dotenv-load

lint:
    cargo clippy --all --all-features -- -W clippy::pedantic

fmt:
    cargo +nightly fmt --all

@docker-build VERSION="latest":
    docker build -f ./deployment/Dockerfile.bot -t desiders/ytdl_tg_bot:{{VERSION}} .

@docker-build-cookie-assignment VERSION="latest":
    docker build -f ./deployment/Dockerfile.cookie_assignment -t desiders/ytdl_tg_bot.cookie_assignment:{{VERSION}} .

@docker-push-dev-bot:
    ./scripts/build-bot-dev-image.sh

@docker-push-dev-downloader:
    ./scripts/build-downloader-dev-image.sh

@docker-push-dev-cookie-assignment:
    ./scripts/build-cookie-assignment-dev-image.sh

@docker-build-downloader VERSION="latest":
    docker build -f ./deployment/Dockerfile.downloader -t desiders/ytdl_tg_bot.downloader:{{VERSION}} .

@docker-build-migration VERSION="latest":
    docker build -f ./migration/Dockerfile -t desiders/ytdl_tg_bot.migration:{{VERSION}} ./migration

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
    helm upgrade bot ./charts/bot -n {{NAMESPACE}}

helm-upgrade-infra NAMESPACE:
    helm upgrade infra ./charts/infra -n {{NAMESPACE}}

helm-install-downloader NAMESPACE:
    helm install downloader ./charts/downloader -n {{NAMESPACE}} --create-namespace

helm-install-cookie-assignment NAMESPACE:
    helm install cookie-assignment ./charts/cookie-assignment -n {{NAMESPACE}} --create-namespace

helm-upgrade-downloader NAMESPACE:
    helm upgrade downloader ./charts/downloader -n {{NAMESPACE}}

helm-upgrade-cookie-assignment NAMESPACE:
    helm upgrade cookie-assignment ./charts/cookie-assignment -n {{NAMESPACE}}

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

k8s-migration NAMESPACE COMMAND="up":
    run_id=$(date +%s); \
    job_name=bot-migration-$run_id; \
    helm template bot ./charts/bot -n {{NAMESPACE}} \
      --show-only templates/migration-job.yaml \
      --set migration.enabled=true \
      --set migration.runId=$run_id \
      --set-string migration.command='{{COMMAND}}' \
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

help:
    just -l
