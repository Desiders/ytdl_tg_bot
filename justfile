set dotenv-load

host := `uname -a`

# Run cargo
run:
    cargo run

# Run release cargo
run-release:
    cargo run --release

# Run docker compose
run-docker:
    docker compose up --build

# Down docker compose
down-docker:
    docker compose down

# Update dependencies
update:
    cargo update

# Clippy
clippy:
    cargo clippy --all --all-features -- -W clippy::pedantic

# Format
format:
    cargo fmt --all -- --check