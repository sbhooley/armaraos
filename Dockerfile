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
RUN cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask
COPY agents ./agents
COPY packages ./packages
RUN cargo build --release --bin openfang

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
EXPOSE 4200
VOLUME /data
ENV OPENFANG_HOME=/data
ENTRYPOINT ["openfang"]
CMD ["start"]
