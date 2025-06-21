set dotenv-load

host := `uname -a`

help:
    just -l

clippy:
    cargo clippy --all --all-features -- -W clippy::pedantic

fmt:
    cargo fmt --all -- --check

@build:
    cargo build --all-features

@build-release:
    cargo build --release --all-features

@run: build
    cargo run

@run-release: build-release
    cargo run --release

@docker-build VERSION="latest":
    docker build -t ytdl_tg_bot:{{VERSION}} .

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
