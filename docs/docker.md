# Docker image

The repository root `Dockerfile` builds a multi-arch image (see `.github/workflows/release.yml`) that runs the `openfang` daemon with Python, Node, and bundled agents. The runtime uses **`OPENFANG_HOME=/data`** and, by default, **`OPENFANG_LISTEN=0.0.0.0:50051`** so the HTTP API is reachable from your host when you publish port **50051** (see below).

## Listen address and port **50051**

The kernel default `api_listen` is **`127.0.0.1:50051`**. Inside Docker, binding only to **127.0.0.1** means **nothing on your Mac/PC can connect** through `-p …:50051` (traffic arrives on the container’s non-loopback interface). The image sets **`OPENFANG_LISTEN=0.0.0.0:50051`**, which overrides `api_listen` at boot so the dashboard is reachable at **`http://localhost:50051/`** when you map the port. Override with **`-e OPENFANG_LISTEN=0.0.0.0:PORT`** if you use a different port.

## Quick start

```bash
docker build -t armaraos:local .
docker run --rm -p 50051:50051 -v armaraos-data:/data \
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

After that, `cargo build --release -p openfang-cli --bin openfang` rebuilds only the CLI and its path dependencies. The cook step uses **`--package openfang-cli`** so the image does **not** compile the Tauri desktop crate (GTK/`gdk-sys`, which would require extra Debian packages). The **`programs/`** tree at the repo root must be copied into the build context: **`openfang-kernel/build.rs`** embeds those AINL files and will panic if the directory is missing.

Together with BuildKit cache (`cache-from` / `cache-to` in CI), this cuts repeated CI and local build time.

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

- **Browser cannot open the dashboard / connection refused** with `-p 50051:50051`: ensure the process listens on **`0.0.0.0`**, not only **`127.0.0.1`** (the stock image sets **`OPENFANG_LISTEN=0.0.0.0:50051`**). Use **`http://localhost:50051/`** on the host (not the container-only URL printed as `127.0.0.1` in logs).
- **Container exits immediately with "Daemon already running"** (or Docker Desktop shows the container stopped): `openfang start` probes for an existing daemon via HTTP. With **`--network host`**, that check can see **another** ArmaraOS on the host. The CLI **skips** this probe when **`/.dockerenv`** is present (normal Docker). If you still see it (e.g. Podman), set **`OPENFANG_SKIP_DAEMON_CHECK=1`** on the container.
- **Link step killed (OOM)** in `docker build`: the release profile uses LTO; increase Docker Desktop memory or pass **`--build-arg LTO=false`** for a quicker, less memory-heavy link.
- **Missing libssl at runtime**: ensure the final stage installs **`libssl3`** (already in the Dockerfile).
