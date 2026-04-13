# ainl-semantic-tagger

> ⚠️ Alpha — API subject to change

Deterministic, heuristic-only semantic tagging and normalization for AINL / ArmaraOS agents.

## What it does

Converts raw text and turn metadata into canonical `SemanticTag` values covering topics, user preferences, tone, correction phrases, and tool names. No ML, no embeddings, no graph-store dependency.

## Crate relationships

- Sits **below** `ainl-graph-extractor` — provides tagging primitives
- Used **by** `ainl-graph-extractor` to normalize episode signals
- Independent of `ainl-memory` — no storage dependency

## Usage

```rust
use ainl_semantic_tagger::tag_turn;

let tags = tag_turn(
    "Please keep it short; we're debugging Rust async.",
    Some("Here is a concise reply."),
    &["file_read".into(), "shell_exec".into()],
);

for tag in tags {
    println!("{} ({})", tag.to_canonical_string(), tag.confidence);
}
```

## License

MIT OR Apache-2.0
