set dotenv-load

host := `uname -a`

help:
    just -l

lint:
    cargo clippy --all --all-features -- -W clippy::pedantic

fmt:
    cargo fmt --all -- --check

@docker-compose-build:
    docker compose build

@docker-up:
    docker compose up

@docker-up-build: docker-compose-build
    docker compose up

@docker-down:
    docker compose down

docker-pull USER VERSION="latest":
    docker pull {{USER}}/ytdl_tg_bot:{{VERSION}}

docker-push USER VERSION="latest":
    @just docker-compose-build
    docker push {{USER}}/ytdl_tg_bot:{{VERSION}}

@docker-db-up:
    docker compose up -d postgres

@docker-db-stop:
    docker compose stop postgres
