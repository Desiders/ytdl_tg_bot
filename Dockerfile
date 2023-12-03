FROM debian:buster-slim AS base
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && apt-get install -y --no-install-recommends ffmpeg \
    && apt-get install -y --no-install-recommends \
        libpq5 build-essential zlib1g-dev libncurses5-dev libgdbm-dev libssl-dev libreadline-dev libffi-dev libbz2-dev libsqlite3-dev \
        wget \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .

ENV PYTHON_VERSION 3.11.6

RUN mkdir -p /usr/src/python
RUN wget -O python.tar.xz "https://www.python.org/ftp/python/${PYTHON_VERSION%%[a-z]*}/Python-$PYTHON_VERSION.tar.xz"
RUN tar --extract --directory /usr/src/python --strip-components=1 --file python.tar.xz \
    && rm python.tar.xz
RUN cd /usr/src/python \
    && ./configure \
    && make -j8 python \
    && make install \
    && cd / \
    && rm -rf /usr/src/python \
    && apt-get purge -y --auto-remove -o APT::AutoRemove::RecommendsImportant=false

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