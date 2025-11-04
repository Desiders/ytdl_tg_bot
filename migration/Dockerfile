FROM alpine:3.20 AS base

FROM rust:1.91-alpine3.20 AS build-deps
RUN apk add --no-cache musl-dev upx
WORKDIR /usr/src/app
COPY Cargo.toml Cargo.lock ./
# Create a dummy `main.rs` to allow cargo to cache dependencies
RUN mkdir src
RUN echo "fn main() {}" > src/main.rs && touch src/lib.rs
RUN cargo build --release --target x86_64-unknown-linux-musl --locked

FROM build-deps AS build-src
WORKDIR /usr/src/app
COPY ./src ./src
RUN touch src/main.rs && touch src/lib.rs
RUN cargo build --release --target x86_64-unknown-linux-musl --locked
RUN upx --best --lzma target/x86_64-unknown-linux-musl/release/migration

FROM base AS final
WORKDIR /app
COPY --from=build-src /usr/src/app/target/x86_64-unknown-linux-musl/release/migration .
ENV RUST_BACKTRACE=full
ENTRYPOINT ["/app/migration"]
