# syntax=docker/dockerfile:1

# ── Dependency recipe (re-runs only when Cargo manifests / Cargo.lock change) ──
FROM lukemathwalker/cargo-chef:latest-rust-1-bookworm AS planner
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask
COPY agents ./agents
COPY packages ./packages
RUN cargo chef prepare --recipe-path recipe.json

# ── Build dependency crates (cached), then workspace crates ──
FROM lukemathwalker/cargo-chef:latest-rust-1-bookworm AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
# Workspace enables openssl "vendored" (compile OpenSSL from source). That path needs perl/make/gcc;
# rust:slim does not ship them. Link against Debian libssl instead (runtime installs libssl3).
ENV OPENSSL_NO_VENDOR=1
COPY --from=planner /build/recipe.json recipe.json
# Defaults: thin LTO + parallel codegen — faster compile/link than fat LTO + 1 unit, same behavior.
# For maximum binary optimization: docker build --build-arg LTO=true --build-arg CODEGEN_UNITS=1 .
ARG LTO=thin
ARG CODEGEN_UNITS=16
ENV CARGO_PROFILE_RELEASE_LTO=${LTO} \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=${CODEGEN_UNITS}
# Only the CLI binary is shipped; cooking the whole workspace would pull GTK (openfang-desktop) and fail without libgdk.
RUN cargo chef cook --release --recipe-path recipe.json --package openfang-cli
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask
COPY agents ./agents
COPY packages ./packages
# openfang-kernel build.rs embeds ../../programs (AINL bundles); omitting this panics.
COPY programs ./programs
RUN cargo build --release -p openfang-cli --bin openfang

FROM rust:1-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    python3 \
    python3-pip \
    python3-venv \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/openfang /usr/local/bin/
COPY --from=builder /build/agents /opt/openfang/agents
# Default api_listen is 127.0.0.1 — that does NOT accept traffic from Docker port publishing. Kernel honors OPENFANG_LISTEN (see openfang-kernel).
EXPOSE 50051
VOLUME /data
ENV OPENFANG_HOME=/data
ENV OPENFANG_LISTEN=0.0.0.0:50051
ENTRYPOINT ["openfang"]
CMD ["start"]
