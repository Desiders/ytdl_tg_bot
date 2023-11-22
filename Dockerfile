FROM debian:buster-slim AS base
RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg \
    && apt-get install -y --no-install-recommends python3 \
    && apt-get install -y --no-install-recommends libpq5 \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .

FROM rust:1.71-buster AS build
WORKDIR /usr/src/app
RUN USER=root cargo init
COPY ./Cargo.toml .
RUN cargo build --release
COPY ./src ./src
COPY phantom.mp3 .
COPY phantom.mp4 .
# https://users.rust-lang.org/t/dockerfile-with-cached-dependencies-does-not-recompile-the-main-rs-file/21577
RUN touch src/main.rs && cargo build --release

FROM base AS final
WORKDIR /app
COPY --from=build /usr/src/app/target/release/ytdl_tg_bot .
ENV RUST_BACKTRACE=full
ENTRYPOINT ["/app/ytdl_tg_bot"]