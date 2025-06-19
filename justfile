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

@docker-run VERSION="latest":
    docker run --rm -it \
        --name ytdl_tg_bot.bot \
        --env-file .env \
        -v ./yt-dlp:/app/yt-dlp \
        -v ./config.toml:/app/config.toml \
        ytdl_tg_bot:{{VERSION}}

@docker-stop:
    docker stop ytdl_tg_bot.bot

@docker-rm: docker-stop
    docker rm ytdl_tg_bot.bot

docker VERSION="latest":
    @just docker-stop >/dev/null 2>&1 || true
    @just docker-rm >/dev/null 2>&1 || true
    @just docker-run {{VERSION}}

@docker-up:
    docker compose up

@docker-up-build:
    docker compose up --build

@docker-down:
    docker compose down

@docker-rmi VERSION="latest": docker-down
    docker rmi ytdl_tg_bot:{{VERSION}}

docker-tag-to-locale USER VERSION="latest" :
    docker tag {{USER}}/ytdl_tg_bot:{{VERSION}} ytdl_tg_bot:{{VERSION}}

docker-pull USER VERSION="latest":
    docker pull {{USER}}/ytdl_tg_bot:{{VERSION}}
    @just docker-tag-to-locale {{USER}} {{VERSION}}

docker-tag-to-remote USER VERSION="latest":
    docker tag ytdl_tg_bot:{{VERSION}} {{USER}}/ytdl_tg_bot:{{VERSION}}

docker-push USER VERSION="latest":
    @just docker-build {{VERSION}}
    @just docker-tag-to-remote {{USER}} {{VERSION}}
    docker push {{USER}}/ytdl_tg_bot:{{VERSION}}
