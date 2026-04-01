# Docker image

The repository root `Dockerfile` builds a multi-arch image (see `.github/workflows/release.yml`) that runs the `openfang` daemon with Python, Node, and bundled agents. The runtime listens on port **4200** and uses **`OPENFANG_HOME=/data`** by default (mount a volume for persistent config).

## Quick start

```bash
docker build -t armaraos:local .
docker run --rm -p 4200:4200 -v armaraos-data:/data \
  -e GROQ_API_KEY="your-key" \
  armaraos:local start
```

Prebuilt images: `ghcr.io/<owner>/armaraos:latest` and version tags (see releases).

## Why `OPENSSL_NO_VENDOR`?

The workspace uses the `openssl` crate with **`features = ["vendored"]`**, which normally compiles OpenSSL from source. That requires Perl, a C toolchain, and a long compile step. The **builder** image uses **`ENV OPENSSL_NO_VENDOR=1`** so the build links against **Debian’s OpenSSL** (`libssl-dev`) instead. The **runtime** stage installs **`libssl3`** so the binary can load `libssl` at run time.

Behavior is unchanged: TLS for IMAP/SMTP and other `native-tls` uses the same OpenSSL APIs, just dynamically linked to the distro libraries inside the container.

## Faster rebuilds: `cargo-chef`

The Dockerfile uses [**cargo-chef**](https://github.com/LukeMathWalker/cargo-chef) in two stages:

1. **Planner** — `cargo chef prepare` writes a **recipe** from your manifests and lockfile.
2. **Builder** — `cargo chef cook --release` compiles **dependencies** only. That layer is cached until `Cargo.lock` or crate manifests change.

After that, `cargo build --release --bin openfang` rebuilds only workspace crates when source changes. Together with BuildKit cache (`cache-from` / `cache-to` in CI), this cuts repeated CI and local build time.

## Release profile defaults (compile time vs. binary size)

The image defaults to:

| Build arg | Default | Effect |
|-----------|---------|--------|
| `LTO` | `thin` | Link-time optimization without full fat LTO; much faster link step than `true` / `fat`. |
| `CODEGEN_UNITS` | `16` | Parallel codegen; faster compile than `1` (workspace default for release is `1` for smallest binary). |

For a **smaller or more aggressively optimized** binary inside Docker (at the cost of slower build/link):

```bash
docker build -t armaraos:max --build-arg LTO=true --build-arg CODEGEN_UNITS=1 .
```

## Multi-arch builds

CI uses `docker buildx` with `--platform linux/amd64,linux/arm64`. ARM64 builds may run under QEMU emulation on `ubuntu-latest` and are slower than native AMD64; caching and `cargo-chef` still help.

## First-time run (`openfang init`)

The image entrypoint runs **`openfang start`**. If **`/data/config.toml`** is missing, the kernel still boots using **defaults** (same as a fresh install). Set at least one LLM API key in the environment (for example **`GROQ_API_KEY`**) or add a mounted **`config.toml`** so agents can call a model.

## Troubleshooting

- **Container exits immediately with "Daemon already running"** (or Docker Desktop shows the container stopped): `openfang start` normally checks whether something already answers on `127.0.0.1:4200`. With **`--network host`**, or in odd setups, that check can hit **another** ArmaraOS instance (often on the host). The CLI **skips** this probe when **`/.dockerenv`** is present (normal Docker). If you still see it (e.g. Podman), set **`OPENFANG_SKIP_DAEMON_CHECK=1`** on the container.
- **Link step killed (OOM)** in `docker build`: the release profile uses LTO; increase Docker Desktop memory or pass **`--build-arg LTO=false`** for a quicker, less memory-heavy link.
- **Missing libssl at runtime**: ensure the final stage installs **`libssl3`** (already in the Dockerfile).
