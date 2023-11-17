set dotenv-load

host := `uname -a`

# Build cargo
build:
    cargo build

# Build release
build-release:
    cargo build --release

# Run cargo
run:
    cargo run

# Run release cargo
run-release:
    cargo run --release

# Build and run cargo
build-run:
    cargo run -- --build

# Build release and run cargo
build-run-release:
    cargo run -- --build --release

# Update dependencies
update:
    cargo update

# Clippy
clippy:
    cargo clippy --all --all-features -- -W clippy::pedantic
