# GA Sign-Off Checklist (Graph Memory Roadmap)

Use this checklist to close the final human approvals required for GA.

This document is intentionally procedural: every bullet has exact actions,
evidence to collect, and a pass/fail outcome.

## Prerequisites (Run Once Before Any Sign-Off)

1. Start from a clean working tree for the release candidate commit.
2. Build and run the daemon from that exact commit.
3. Record metadata at the top of your evidence notes:
   - `version` (from `/api/status`)
   - `commit_sha` (from git)
   - `review_date`
   - `reviewer_name`

### Commands

```bash
cd /Users/clawdbot/.openclaw/workspace/armaraos
git rev-parse HEAD
cargo build -p openfang-cli
cargo run -p openfang-cli -- start
```

In a second terminal:

```bash
BASE="http://127.0.0.1:50051"
curl -s "$BASE/api/status"
bash scripts/check-memory-ga-gates.sh --base "$BASE"
```

## 1) Product Owner Sign-Off (Controls UX + Policy Semantics)

### 1.1 Settings memory controls are understandable and usable

- **Step 1:** Open dashboard `Settings`.
- **Step 2:** Locate `Graph memory controls`.
- **Step 3:** Select at least 2 agents and verify controls load per agent.
- **Step 4:** Toggle each control one at a time and save:
  - Memory enabled
  - Temporary mode
  - Shared memory enabled
  - Episodic/Semantic/Conflict/Procedural block toggles
- **Step 5:** Refresh page and verify values persisted.

**Expected pass result**
- Controls save without error.
- Values persist across refresh.
- Agent A changes do not unintentionally overwrite Agent B.

**Evidence**
- Screenshots before/after save for two agents.
- API snapshots:
  - `GET /api/graph-memory/controls?agent_id=<id>` for each agent.

### 1.2 Graph Memory controls (`remember`, `forget`, `inspect`, `clear_scope`) are clear

- **Step 1:** Open `Graph Memory` page.
- **Step 2:** Run `remember` with a test fact in `agent_private`.
- **Step 3:** Run `inspect` for that scope and verify fact appears.
- **Step 4:** Run `forget` for that fact and verify removal.
- **Step 5:** Add 2 test facts, then run `clear_scope` and verify empty scope.

**Expected pass result**
- Each operation produces visible, deterministic result.
- No silent failure or ambiguous UI state.

**Evidence**
- Screenshot or log for each operation.
- API payload/response examples used during the run.

### 1.3 Overview + Chat indicators are useful but low-noise

- **Step 1:** Open `Overview` and verify graph memory card shows:
  - injected lines/truncations/skipped
  - provenance + contradiction gate status
- **Step 2:** In `Chat`, send one message and verify the subtle memory turn indicator appears (`memory on turn` or `memory off turn`).
- **Step 3:** Send another message and confirm indicator updates, not duplicated/noisy.

**Expected pass result**
- Indicators are visible, understandable, and not intrusive.

**Evidence**
- One screenshot from `Overview`.
- One screenshot from `Chat` telemetry strip.

### 1.4 Progressive disclosure and help text semantics

- **Step 1:** Verify default UX surfaces only high-level controls.
- **Step 2:** Verify advanced diagnostics are available in `Graph Memory` (why-selected drawer, contradiction panel).
- **Step 3:** Confirm tooltips/help text mention:
  - temporary mode semantics (reads+writes disabled)
  - scope semantics (`agent_private`, `workspace_shared`, `org_shared`)
  - forget vs clear-scope behavior.

**Expected pass result**
- New users can operate basics without deep internals.
- Operators can access diagnostics without hidden workflows.

**Evidence**
- Brief reviewer note (2-4 sentences) on clarity/usability.

---

## 2) Runtime Owner Sign-Off (Reliability + Performance Gates)

### 2.1 Control plane reliability and safety

- **Step 1:** Run offline gate script.
- **Step 2:** Run live gate script against active daemon.
- **Step 3:** Confirm `/api/status` includes graph memory gate keys.

### Commands

```bash
cd /Users/clawdbot/.openclaw/workspace/armaraos
bash scripts/check-memory-ga-gates.sh --offline
BASE="http://127.0.0.1:50051"
bash scripts/check-memory-ga-gates.sh --base "$BASE"
curl -s "$BASE/api/status"
```

**Expected pass result**
- Both scripts pass.
- `provenance_gate_pass` and `contradiction_gate_pass` are present and true.

**Evidence**
- Raw command outputs attached to the sign-off artifact.

### 2.2 Temporary mode enforces no reads and no writes

- **Step 1:** Enable temporary mode for target agent in Settings or API.
- **Step 2:** Run at least one chat turn for that agent.
- **Step 3:** Confirm memory suppression counters increase in `/api/status`.

### API shortcut

```bash
curl -s -X PUT "$BASE/api/graph-memory/controls" \
  -H "Content-Type: application/json" \
  -d '{"agent_id":"<AGENT_ID>","temporary_mode":true}'
```

**Expected pass result**
- `temp_mode_suppressed_reads_total` and/or `temp_mode_suppressed_writes_total` increase.
- No observed memory injection lines for that agent while temporary mode is on.

**Evidence**
- Before/after `/api/status` metric snapshots.

### 2.3 Performance budget check (latency/tokens/cost)

- **Step 1:** Define workload (minimum 20 representative prompts).
- **Step 2:** Run baseline cohort (memory disabled).
- **Step 3:** Run treatment cohort (default memory settings enabled).
- **Step 4:** Compare:
  - median turn latency
  - tokens in/out
  - cost per turn

**Expected pass result**
- No budget-breaking regressions.
- Any increase is within agreed threshold and justified by quality gains.

**Evidence**
- Benchmark summary table in sign-off notes (CSV or markdown is fine).

---

## 3) Security/Privacy Owner Sign-Off (Retention + Scope + Auditability)

### 3.1 Scope boundaries are enforced

- **Step 1:** Insert fact in `agent_private` for Agent A.
- **Step 2:** Attempt recall/inspect from Agent B without shared scope enabled.
- **Step 3:** Repeat with `workspace_shared` enabled and validate expected visibility.

**Expected pass result**
- No cross-scope leakage without policy allowance.
- Shared scope behavior matches documented policy.

**Evidence**
- API responses proving blocked and allowed paths.

### 3.2 Deletion semantics and SLA

- **Step 1:** Add facts to a known scope.
- **Step 2:** Execute `forget` and `clear_scope`.
- **Step 3:** Immediately run `inspect` and verify removed facts are absent.
- **Step 4:** Run one turn and verify removed facts are not injected.

**Expected pass result**
- Deletion effect is visible within documented SLA window.
- No stale deleted facts recalled after SLA.

**Evidence**
- API timeline (timestamps + responses) demonstrating deletion propagation.

### 3.3 Auditability for promotions/deletions/policy denials

- **Step 1:** Perform a promotion/share action, a deletion action, and a denied action.
- **Step 2:** Verify corresponding audit entries exist.
- **Step 3:** Confirm record includes actor, action, scope, and outcome.

**Expected pass result**
- Audit records are complete and queryable for each critical action.

**Evidence**
- Extracted audit rows or screenshot with identifiers redacted as needed.

---

## 4) Data/ML Owner Sign-Off (Evaluation Validity + A/B Interpretation)

### 4.1 Evaluation design validity

- **Step 1:** Freeze prompt set and scoring rubric before runs.
- **Step 2:** Declare acceptance thresholds upfront:
  - task completion uplift target
  - contradiction incidence max
  - repeated failure loop reduction target
- **Step 3:** Validate no prompt leakage between cohorts.

**Expected pass result**
- Evaluation is pre-registered and reproducible.

**Evidence**
- One-page methodology note checked into docs or attached to release ticket.

### 4.2 Quality outcomes (baseline vs memory-enabled)

- **Step 1:** Run baseline (memory off) and treatment (memory on) using same dataset.
- **Step 2:** Compute and compare:
  - completion rate
  - contradiction rate
  - repeated tool-failure loop rate
- **Step 3:** Spot-check failures for false positives in scoring.

**Expected pass result**
- Quality metrics meet or exceed threshold rule from roadmap.

**Evidence**
- Metrics summary + raw run artifact location.

### 4.3 Rollout decision and stop conditions

- **Step 1:** Validate rollout controls are operable (`internal`, `opt-in`, `default` paths as configured).
- **Step 2:** Confirm kill switches work live (global and per-block).
- **Step 3:** Document promotion criteria and rollback triggers.

**Expected pass result**
- Clear go/no-go criteria with immediate rollback path.

**Evidence**
- Short decision memo: `Go`, `Go with guardrails`, or `Hold`.

---

## Final Sign-Off Record (Fill One Per Owner)

Copy this section for each approver.

```text
Owner:
Role:
Date:
Commit SHA:
Build Version:

Checklist sections reviewed:
- [ ]
- [ ]
- [ ]

Decision:
- [ ] Approve
- [ ] Approve with conditions
- [ ] Reject

Conditions / required follow-ups:

Evidence links/attachments:

Signature (name):
```

## GA Exit Rule

GA is approved only when all four owner sign-offs are in `Approve` or
`Approve with conditions` state and all conditions have explicit owners/dates.
