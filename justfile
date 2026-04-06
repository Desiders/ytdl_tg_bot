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

helm-install-bot NAMESPACE:
    helm install bot ./charts/bot -n {{NAMESPACE}} --create-namespace

helm-upgrade-bot NAMESPACE:
    helm upgrade bot ./charts/bot -n {{NAMESPACE}}

helm-install-downloader NAMESPACE:
    helm install downloader ./charts/downloader -n {{NAMESPACE}} --create-namespace

helm-upgrade-downloader NAMESPACE:
    helm upgrade downloader ./charts/downloader -n {{NAMESPACE}}

scale-downloader NAMESPACE REPLICAS="1":
    helm upgrade downloader ./charts/downloader -n {{NAMESPACE}} --set downloader.replicas={{REPLICAS}}

k8s-rollout-bot NAMESPACE:
    kubectl rollout restart deployment/bot -n {{NAMESPACE}}

k8s-rollout-downloader NAMESPACE:
    kubectl rollout restart deployment/downloader -n {{NAMESPACE}}

k8s-update-bot-config NAMESPACE:
    kubectl create secret generic bot-config --from-file=config.toml=./configs/config.toml --dry-run=client -o yaml | kubectl apply -n {{NAMESPACE}} -f -
    just k8s-rollout-bot {{NAMESPACE}}

k8s-sync-bot-cookies NAMESPACE SECRET_NAME="bot-cookies" SOURCE_DIR="cookies":
    NAMESPACE={{NAMESPACE}} SECRET_NAME={{SECRET_NAME}} ./scripts/sync-cookies-secret.sh {{SOURCE_DIR}}

k8s-update-downloader-config NAMESPACE:
    kubectl create secret generic downloader-config --from-file=downloader.toml=./configs/downloader.toml --dry-run=client -o yaml | kubectl apply -n {{NAMESPACE}} -f -
    just k8s-rollout-downloader {{NAMESPACE}}

k8s-migration NAMESPACE VERSION COMMAND:
    kubectl run db-migration --image=desiders/ytdl_tg_bot.migration:{{VERSION}} --env="DATABASE_URL=postgres://admin:admin@postgres-rw:5432/api" --restart=Never --rm -it -n {{NAMESPACE}} -- {{COMMAND}}

k8s-logs-bot NAMESPACE:
    kubectl logs -l app=bot -n {{NAMESPACE}} -f

k8s-logs-downloader NAMESPACE:
    kubectl logs -l app=downloader -n {{NAMESPACE}} -f

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
