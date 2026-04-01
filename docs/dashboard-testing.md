# Dashboard testing notes

## Kernel SSE (`GET /api/events/stream`)

- **Smoke:** Open the dashboard, confirm the sidebar **SSE** badge appears when the stream connects (kernel running, same origin / loopback as usual).
- **API:** `cargo test -p openfang-api --test api_integration_test test_kernel_events_stream_sse_smoke` and `cargo test -p openfang-api --test sse_stream_auth` cover HTTP behavior (including loopback vs remote auth).

## Overview refresh

- On **Overview**, lifecycle/system kernel events trigger a **debounced** refresh (~400ms) via `armaraos-kernel-event`. The page also shows a **Last kernel event** line when `kernelEvents.last` is set.
- **Page leave:** The overview component registers `@page-leave.window="stopAutoRefresh()"` so timers and kernel listeners are cleared when navigating away. If you add **Playwright** (or similar) later, assert that after switching to another hash/route, `setInterval`-driven refresh is not still firing (e.g. spy on `/api/usage` or equivalent after leaving Overview).
