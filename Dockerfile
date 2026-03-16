FROM alpine:3.22 AS base
RUN apk add --no-cache \
        ffmpeg \
        python3 \
        deno \
        bash \
        curl \
    && rm -rf /var/cache/apk/*

FROM rust:1.91 AS build
RUN apt-get update && apt-get install -y musl-tools upx
RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /usr/src/app

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main(){}" > src/main.rs
RUN cargo fetch --locked

COPY ./src ./src
RUN touch src/main.rs
RUN cargo build --release --target x86_64-unknown-linux-musl --locked
RUN upx --best --lzma target/x86_64-unknown-linux-musl/release/ytdl_tg_bot

FROM base AS final
WORKDIR /app
VOLUME ["/app/yt-dlp", "/app/config.toml", "/app/cookies"]
COPY --from=build /usr/src/app/target/x86_64-unknown-linux-musl/release/ytdl_tg_bot .
ENV RUST_BACKTRACE=full
ENTRYPOINT ["/app/ytdl_tg_bot"]
