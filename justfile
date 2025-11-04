set dotenv-load

host := `uname -a`

help:
    just -l

lint:
    cargo clippy --all --all-features -- -W clippy::pedantic

fmt:
    cargo fmt --all -- --check

@docker-build:
    docker compose --profile default build

@docker-migration-build:
    docker compose build migration

@docker-up:
    docker compose --profile default up

@docker-up-build: docker-build
    docker compose --profile default up

@docker-down:
    docker compose --profile default down

@docker-dev-build:
    docker compose --profile dev build

@docker-dev-up:
    docker compose --profile dev up -d && docker compose --profile dev logs -f

@docker-dev-up-build: docker-dev-build
    docker compose --profile dev up -d && docker compose --profile dev logs -f

@docker-dev-down:
    docker compose --profile dev down

@docker-migration COMMAND:
    docker compose run --rm migration {{COMMAND}}

@docker-migration-with-build COMMAND:
    @just docker-migration-build
    docker compose run --rm migration {{COMMAND}}

@run:
    cargo run

docker-pull USER VERSION="latest":
    docker pull {{USER}}/ytdl_tg_bot:{{VERSION}}

docker-push USER VERSION="latest":
    @just docker-build
    docker push {{USER}}/ytdl_tg_bot:{{VERSION}}

docker-migration-pull USER VERSION="latest":
    docker pull {{USER}}/ytdl_tg_bot.migration:{{VERSION}}

docker-migration-push USER VERSION="latest":
    @just docker-migration-build
    docker push {{USER}}/ytdl_tg_bot.migration:{{VERSION}}

@docker-db-up:
    docker compose up -d postgres

@docker-db-stop:
    docker compose stop postgres
