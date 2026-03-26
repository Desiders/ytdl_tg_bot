set dotenv-load

host := `uname -a`
compose := "docker compose --env-file .env -f deployment/docker-compose.yaml"

help:
    just -l

lint:
    cargo clippy --all --all-features -- -W clippy::pedantic

fmt:
    cargo +nightly fmt --all

@docker-build:
    {{compose}} --profile default build

@docker-build-downloader:
    {{compose}} --profile downloader build

@docker-migration-build:
    {{compose}} build migration

@docker-up:
    {{compose}} --profile default up

@docker-up-build: docker-build
    {{compose}} --profile default up

@docker-up-downloader:
    {{compose}} --profile default --profile downloader up

@docker-up-build-downloader: docker-build-downloader
    {{compose}} --profile downloader up

@docker-down:
    {{compose}} down

@docker-dev-build:
    {{compose}} --profile dev build

@docker-dev-up:
    {{compose}} --profile dev up -d && {{compose}} --profile dev logs -f

@docker-dev-up-build: docker-dev-build
    {{compose}} --profile dev up -d && {{compose}} --profile dev logs -f

@docker-dev-down:
    {{compose}} --profile dev down

@docker-migration COMMAND:
    {{compose}} run --rm migration {{COMMAND}}

@docker-migration-with-build COMMAND:
    @just docker-migration-build
    {{compose}} run --rm migration {{COMMAND}}

@run:
    cargo run -p ytdl_tg_bot

@run-downloader:
    cargo run -p ytdl_tg_downloader

docker-pull USER VERSION="latest":
    docker pull {{USER}}/ytdl_tg_bot:{{VERSION}}

docker-push USER VERSION="latest":
    @just docker-build
    docker push {{USER}}/ytdl_tg_bot:{{VERSION}}

docker-pull-downloader USER VERSION="latest":
    docker pull {{USER}}/ytdl_tg_bot.downloader:{{VERSION}}

docker-push-downloader USER VERSION="latest":
    @just docker-build-downloader
    docker push {{USER}}/ytdl_tg_bot.downloader:{{VERSION}}

docker-migration-pull USER VERSION="latest":
    docker pull {{USER}}/ytdl_tg_bot.migration:{{VERSION}}

docker-migration-push USER VERSION="latest":
    @just docker-migration-build
    docker push {{USER}}/ytdl_tg_bot.migration:{{VERSION}}

@docker-db-up:
    {{compose}} up -d postgres

@docker-db-stop:
    {{compose}} stop postgres
