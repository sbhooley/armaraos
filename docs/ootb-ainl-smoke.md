# OOTB AINL — manual smoke

Use this after install or when verifying embedded programs and curated cron. Requires a running API. Default bind in a fresh install is **`http://127.0.0.1:50051`** (`api_listen` in `~/.armaraos/config.toml`); older docs may cite `4200` — use whatever `openfang start` prints.

## 1. Files on disk

After the kernel has booted at least once:

```bash
ls ~/.armaraos/ainl-library/armaraos-programs/armaraos_health_ping/
cat ~/.armaraos/ainl-library/armaraos-programs/.embedded-revision.txt
```

You should see `armaraos_health_ping.ainl` and a revision line matching `EMBEDDED_PROGRAMS_REVISION` in `crates/openfang-kernel/src/embedded_ainl_programs.rs`.

## 2. Scheduler

Open **Dashboard → Scheduler**. You should see a job named **`armaraos-ainl-health-weekly`** (type AINL), **Active**, on a weekly Sunday schedule—if curated registration ran and the job was added. If missing, use **Register curated cron** on that page (or boot again without `ARMARAOS_DISABLE_CURATED_AINL_CRON=1`).

Optional **curl** check (same data as the Scheduler page):

```bash
curl -s "http://127.0.0.1:50051/api/cron/jobs" | python3 -c "import sys,json; d=json.load(sys.stdin); print([j.get('name') for j in d.get('jobs',[]) if 'armaraos' in (j.get('name') or '')])"
```

You should see **`armaraos-ainl-health-weekly`** in the list (plus other curated jobs depending on `curated_ainl_cron.json`).

## 3. Register-curated API

```bash
curl -s -X POST http://127.0.0.1:50051/api/ainl/library/register-curated \
  -H "Content-Type: application/json" \
  -d '{}'
```

Expect JSON including **`registered`** (number of newly added catalog jobs this run) and **`embedded_programs_written`** (files updated under `armaraos-programs/`). First run after an upgrade may show a non-zero `embedded_programs_written`; idempotent re-runs often show `0`.

If `api_key` is set in config, add `Authorization: Bearer <key>` (or the same token your dashboard uses).
