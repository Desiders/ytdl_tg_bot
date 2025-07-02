FROM alpine:3.18 AS base
RUN apk add --no-cache \
        ffmpeg \
        python3 \
        bash \
        curl \
    && rm -rf /var/cache/apk/*

FROM rust:1.87-slim-bullseye AS build-deps
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        musl-tools \
        pkg-config \
        build-essential \
        upx-ucl \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get purge -y --auto-remove -o APT::AutoRemove::RecommendsImportant=false
RUN rustup target add x86_64-unknown-linux-musl
WORKDIR /usr/src/app
COPY Cargo.toml Cargo.lock ./
# Create a dummy `main.rs` to allow cargo to cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --target x86_64-unknown-linux-musl --locked

FROM build-deps AS build-src
WORKDIR /usr/src/app
COPY ./src ./src
RUN touch src/main.rs
RUN cargo build --release --target x86_64-unknown-linux-musl --locked
RUN upx --best --lzma target/x86_64-unknown-linux-musl/release/ytdl_tg_bot

FROM base AS final
WORKDIR /app
VOLUME ["/app/yt-dlp"]
VOLUME ["/app/config.toml"]
VOLUME ["/app/cookies"]
COPY --from=build-src /usr/src/app/target/x86_64-unknown-linux-musl/release/ytdl_tg_bot .
ENV RUST_BACKTRACE=full
ENTRYPOINT ["/app/ytdl_tg_bot"]
