set dotenv-load

lint:
    cargo clippy --all --all-features -- -W clippy::pedantic

fmt:
    cargo +nightly fmt --all

@docker-build VERSION="latest":
    docker build -f ./deployment/Dockerfile.bot -t desiders/ytdl_tg_bot:{{VERSION}} .

@docker-push-dev-bot:
    ./scripts/build-bot-dev-image.sh

@docker-push-dev-downloader:
    ./scripts/build-downloader-dev-image.sh

@docker-build-downloader VERSION="latest":
    docker build -f ./deployment/Dockerfile.downloader -t desiders/ytdl_tg_bot.downloader:{{VERSION}} .

@docker-build-migration VERSION="latest":
    docker build -f ./deployment/Dockerfile.migration -t desiders/ytdl_tg_bot.migration:{{VERSION}} .

docker-push USER VERSION="latest":
    @just docker-build {{VERSION}}
    docker push {{USER}}/ytdl_tg_bot:{{VERSION}}

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

helm-install-bot:
    helm install bot ./charts/bot -n bot --create-namespace

helm-upgrade-bot:
    helm upgrade bot ./charts/bot -n bot

helm-install-downloader:
    helm install downloader ./charts/downloader -n downloader --create-namespace

helm-upgrade-downloader:
    helm upgrade downloader ./charts/downloader -n downloader

scale-downloader REPLICAS="1":
    helm upgrade downloader ./charts/downloader -n downloader --set downloader.replicas={{REPLICAS}}

k8s-rollout-bot:
    kubectl rollout restart deployment/bot -n bot

k8s-rollout-downloader:
    kubectl rollout restart deployment/downloader -n downloader

k8s-update-bot-config:
    kubectl create secret generic bot-config --from-file=config.toml=./configs/config.toml --dry-run=client -o yaml | kubectl apply -n bot -f -
    just k8s-rollout-bot

k8s-sync-bot-cookies:
    NAMESPACE=bot SECRET_NAME=bot-cookies ./scripts/sync-cookies-secret.sh cookies

k8s-update-downloader-config:
    kubectl create secret generic downloader-config --from-file=downloader.toml=./configs/downloader.toml --dry-run=client -o yaml | kubectl apply -n downloader -f -
    just k8s-rollout-downloader

k8s-migration VERSION COMMAND:
    kubectl run db-migration --image=desiders/ytdl_tg_bot.migration:{{VERSION}} --env="DATABASE_URL=postgres://admin:admin@postgres-rw:5432/api" --restart=Never --rm -it -n bot -- {{COMMAND}}

k8s-logs-bot:
    kubectl logs -l app=bot -n bot -f

k8s-logs-downloader:
    kubectl logs -l app=downloader -n downloader -f

k8s-logs-db:
    kubectl logs -l cnpg.io/cluster=postgres -n bot -f

k8s-logs-telegram-api:
    kubectl logs -l app=telegram-bot-api -n bot

k8s-logs-yt-toolkit:
    kubectl logs -l app=yt-toolkit-api -n bot

k8s-logs-pot-provider:
    kubectl logs -l app=yt-pot-provider -n downloader

help:
    just -l
