FROM python:3.12-slim-bullseye AS base
RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get purge -y --auto-remove -o APT::AutoRemove::RecommendsImportant=false
WORKDIR /app
COPY . .

FROM rust:1.82-slim-bullseye AS build-dependencies
RUN apt-get update \
    && apt-get install -y --no-install-recommends libssl-dev \
    && apt-get install -y --no-install-recommends pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get purge -y --auto-remove -o APT::AutoRemove::RecommendsImportant=false
WORKDIR /usr/src/app
RUN USER=root cargo init
COPY ./Cargo.toml .
RUN cargo build --release

FROM build-dependencies AS build-source
WORKDIR /usr/src/app
COPY ./src ./src
RUN touch src/main.rs && cargo build --release

FROM base AS final
WORKDIR /app
COPY --from=build-source /usr/src/app/target/release/ytdl_tg_bot .
ENV RUST_BACKTRACE=full
ENTRYPOINT ["/app/ytdl_tg_bot"]
