# skill-mint-stub

Minimal AINL graph: builds `# {intent}` + `## Episode` + `episode` text from the learning frame v1 `in:` keys.

- Full Markdown (Meta, refs, tags): `openfang_kernel::skills_staging::render_skill_draft_markdown` and `POST /api/learning/skill-draft`
- Spec: [docs/learning-frame-v1.md](../../docs/learning-frame-v1.md)

```bash
ainl validate programs/skill-mint-stub/skill_mint_stub.ainl --strict
ainl run programs/skill-mint-stub/skill_mint_stub.ainl --json \
  --frame-json @programs/learning-frame-echo/frame.example.json
```
