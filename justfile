set dotenv-load

host := `uname -a`

help:
    just -l

lint:
    cargo clippy --all --all-features -- -W clippy::pedantic

fmt:
    cargo fmt --all -- --check

@docker-build:
    docker compose build

@docker-up:
    docker compose up

@docker-up-build: docker-build
    docker compose up

@docker-down:
    docker compose down

@docker-dev-build:
    docker compose --profile dev build

@docker-dev-up:
    docker compose --profile dev up -d && docker compose --profile dev logs -f

@docker-dev-up-build: docker-dev-build
    docker compose --profile dev up -d && docker compose --profile dev logs -f

@docker-dev-down:
    docker compose --profile dev down

@run:
    cargo run

docker-pull USER VERSION="latest":
    docker pull {{USER}}/ytdl_tg_bot:{{VERSION}}

docker-push USER VERSION="latest":
    @just docker-build
    docker push {{USER}}/ytdl_tg_bot:{{VERSION}}

@docker-db-up:
    docker compose up -d postgres

@docker-db-stop:
    docker compose stop postgres
