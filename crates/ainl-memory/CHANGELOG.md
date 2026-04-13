# Changelog

All notable changes to **ainl-memory** are documented here. This crate follows semantic intent for alphas: minor bumps signal schema or API additions consumers should pin.

## 0.1.3-alpha

### Added

- **`EpisodicNode`**: optional `user_message` and `assistant_response` (`Option<String>`) for offline extractors and richer persona / graph tooling; omitted from JSON when unset (`skip_serializing_if`).
- **`new_episode`**: initializes the new optional fields to `None`.

### Notes for downstream

- Crates.io currently lists **0.1.2-alpha** as latest; any crate that reads episode payloads or constructs `EpisodicNode` literals should bump to **0.1.3-alpha** before publishing dependents that rely on these fields.
- Publish order: **ainl-memory** → **ainl-persona** → **ainl-graph-extractor** → **ainl-runtime** (see `scripts/publish-prep-ainl-crates.sh`).

## 0.1.2-alpha

Prior published baseline on crates.io (semantic recurrence / graph store evolution).
