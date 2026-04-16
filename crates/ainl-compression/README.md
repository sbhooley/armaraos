# ainl-compression

Standalone prompt compression primitives for AINL hosts and external Rust agents.

## Why this crate exists

- Reusable outside ArmaraOS / OpenFang (`cargo add ainl-compression`)
- Minimal dependency surface
- Clear AINL ownership and attribution

## Current scope

- Input prompt compression (`PromptCompressor`)
- Eco modes:
  - `Off`
  - `Balanced`
  - `Aggressive`
- Natural-language mode parsing (`EfficientMode::parse_natural_language`)
- Structured telemetry (`CompressionMetrics`)

Output/dense response compression is intentionally out-of-scope for now.

## Basic usage

```rust
use ainl_compression::{EfficientMode, PromptCompressor};

let compressor = PromptCompressor::new(EfficientMode::Balanced);
let compressed = compressor.compress("Please summarize this long message...");
println!("compressed text: {}", compressed.text);
```

## Telemetry callback

```rust
use ainl_compression::{EfficientMode, PromptCompressor};

let compressor = PromptCompressor::with_telemetry_callback(
    EfficientMode::Balanced,
    Some(Box::new(|m| {
        println!(
            "mode={:?} saved={} ({:.1}%)",
            m.mode, m.tokens_saved, m.savings_ratio_pct
        );
    })),
);

let _ = compressor.compress("Long prompt...");
```

## Optional feature: graph-telemetry

Enable `graph-telemetry` when your host wants to serialize telemetry structures
for graph/event pipelines:

```toml
ainl-compression = { version = "0.1.0-alpha", features = ["graph-telemetry"] }
```

This adds serde derives for shared telemetry structs without coupling this crate
to any specific graph/memory runtime implementation.

## ArmaraOS integration model

- This crate stays runtime-agnostic.
- ArmaraOS/OpenFang can:
  - persist aggregate metrics to `openfang-memory`
  - attach turn-level telemetry into episodic trace metadata in graph memory

That keeps the crate externally reusable while still advancing unified graph
execution tracing inside ArmaraOS.
