# learning-frame-echo

Minimal AINL graph proving **learning frame v1** keys line up with compact `in:` wiring.

- Spec: [docs/learning-frame-v1.md](../../docs/learning-frame-v1.md)

Validate:

```bash
ainl validate programs/learning-frame-echo/learning_frame_echo.ainl --strict
```

Smoke run (from repo root, with `ainl` on PATH). Pass the frame via `--frame-json` (inline or `@file`):

```bash
ainl run programs/learning-frame-echo/learning_frame_echo.ainl --json \
  --frame-json @programs/learning-frame-echo/frame.example.json
```

Expect `"result": "test-1"` (the graph echoes `run_id`).
