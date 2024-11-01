set dotenv-load

host := `uname -a`

# Run cargo
run:
    cargo run

# Run release cargo
run-release:
    cargo run --release

# Run docker compose with build
run-docker-build:
    docker compose up --build

# Run docker compose
run-docker:
    docker compose up

# Down docker compose
down-docker:
    docker compose down

# Build docker compose
build-docker:
    docker compose build

# Update dependencies
update:
    cargo update

# Clippy
clippy:
    cargo clippy --all --all-features -- -W clippy::pedantic

# Format
format:
    cargo fmt --all -- --check

fmt: format
