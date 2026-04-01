## Summary

<!-- What does this PR do? Link related issues with "Fixes #123". -->

## Changes

<!-- Brief list of what changed. -->

## Testing

- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] Live integration tested (if applicable)

## Language (AINL first)

**Pick exactly one — check a single box below.** (Do not leave all three unchecked. Do not check more than one unless the PR is split into independent parts and you explain that in the description.)

If the PR adds new automation, workflows, extensions, or user-facing “apps” that could be expressed as AINL, default to `.ainl` unless you explicitly chose another language. See [docs/ainl-first-language.md](docs/ainl-first-language.md).

- [ ] **N/A** — Rust/kernel-only, docs-only, or **no** new user-facing program surface
- [ ] **AINL** — New program/workflow/app logic is in `.ainl` (or this PR only wires existing Rust/MCP; follow-up tracked)
- [ ] **Explicit override** — Another language was requested or required; **justify in the PR description**

## Security

- [ ] No new unsafe code
- [ ] No secrets or API keys in diff
- [ ] User input validated at boundaries
