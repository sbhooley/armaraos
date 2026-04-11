//! Env-gated HTTP load harness against a running ArmaraOS / OpenFang API daemon.
//!
//! Safety: refuses to run unless `ARMARAOS_LOAD_TEST=1`. See `docs/load-testing.md`.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// CLI overrides for [`run_load_test`].
#[derive(Debug, Clone, Default)]
pub struct LoadTestCli {
    pub base_url: Option<String>,
    pub agent_id: Option<String>,
    pub bearer: Option<String>,
    pub concurrency: Option<u32>,
    pub kv_ops: Option<u32>,
    pub message_rounds: Option<u32>,
    pub message: Option<String>,
    pub workflow_runs: Option<u32>,
    pub inter_batch_ms: Option<u64>,
    pub max_wall_secs: Option<u64>,
    pub config_path: Option<PathBuf>,
    pub dry_run: bool,
}

fn env_trim(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn env_u32(name: &str) -> Option<u32> {
    env_trim(name).and_then(|s| s.parse().ok())
}

fn env_u64(name: &str) -> Option<u64> {
    env_trim(name).and_then(|s| s.parse().ok())
}

fn default_config_path() -> PathBuf {
    if let Ok(h) = std::env::var("ARMARAOS_HOME").or_else(|_| std::env::var("OPENFANG_HOME")) {
        return PathBuf::from(h).join("config.toml");
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".armaraos").join("config.toml")
}

fn read_driver_isolation(config_path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(config_path).ok()?;
    if let Ok(v) = toml::from_str::<toml::Value>(&raw) {
        if let Some(s) = v
            .get("llm")
            .and_then(|t| t.get("driver_isolation"))
            .and_then(|x| x.as_str())
        {
            return Some(s.to_string());
        }
    }
    scan_driver_isolation_table(&raw)
}

fn scan_driver_isolation_table(raw: &str) -> Option<String> {
    let mut in_llm = false;
    for line in raw.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            in_llm = t == "[llm]";
            continue;
        }
        if in_llm {
            let t = line.trim();
            if t.starts_with('[') {
                break;
            }
            if let Some(rest) = t.strip_prefix("driver_isolation") {
                let rest = rest.trim_start();
                if let Some(v) = rest.strip_prefix('=') {
                    let v = v.trim().trim_matches('"').trim_matches('\'');
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    None
}

fn normalize_base_url(s: &str) -> String {
    s.trim_end_matches('/').to_string()
}

fn clamp_concurrency(n: u32) -> u32 {
    if n == 0 {
        eprintln!("warning: concurrency 0 invalid, using 1");
        return 1;
    }
    if n > 128 {
        eprintln!("warning: concurrency {n} clamped to 128");
        return 128;
    }
    n
}

fn build_client(bearer: Option<&str>) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    if let Some(t) = bearer.filter(|s| !s.is_empty()) {
        let val = format!("Bearer {t}")
            .parse()
            .map_err(|_| anyhow!("invalid Bearer token for Authorization header"))?;
        headers.insert(reqwest::header::AUTHORIZATION, val);
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(600))
        .build()
        .context("reqwest::Client::build")
}

async fn get_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let r = client.get(url).send().await.context("GET {url}")?;
    let status = r.status();
    let body = r.text().await.context("read body")?;
    if !status.is_success() {
        bail!("GET {url} -> {status}: {body}");
    }
    Ok(body)
}

async fn get_json(client: &reqwest::Client, url: &str) -> Result<serde_json::Value> {
    let t = get_text(client, url).await?;
    serde_json::from_str(&t).context("parse JSON")
}

async fn post_json(
    client: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<(reqwest::StatusCode, String)> {
    let r = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = r.status();
    let text = r.text().await.context("read POST body")?;
    Ok((status, text))
}

async fn put_json(
    client: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<(reqwest::StatusCode, String)> {
    let r = client
        .put(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(body)
        .send()
        .await
        .with_context(|| format!("PUT {url}"))?;
    let status = r.status();
    let text = r.text().await.context("read PUT body")?;
    Ok((status, text))
}

fn print_llm_metric_lines(metrics_body: &str) {
    println!("--- /api/metrics (llm_* and related) ---");
    for line in metrics_body.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.contains("llm_") {
            println!("{line}");
        }
    }
    println!("--- end llm_* excerpt ---");
}

fn print_isolation_note(config_path: &Path, isolation: Option<&str>) {
    println!(
        "--- [llm] driver isolation (config: {}) ---",
        config_path.display()
    );
    match isolation {
        Some(s) if s.eq_ignore_ascii_case("isolated") => {
            println!("driver_isolation = {s}");
            println!("Note: isolated mode skips the factory LRU — expect distinct driver instances per resolution key, not shared client reuse across keys.");
        }
        Some(s) => {
            println!("driver_isolation = {s} (shared LRU is enabled unless set to isolated)");
        }
        None => {
            println!(
                "Could not read driver_isolation from config (file missing or no [llm] section)."
            );
            println!("Default in types is effectively shared factory mode.");
        }
    }
}

fn agent_exists(agents_json: &serde_json::Value, agent_id: &str) -> bool {
    agents_json
        .as_array()
        .map(|arr| {
            arr.iter().any(|a| {
                a.get("id")
                    .and_then(|v| v.as_str())
                    .map(|id| id == agent_id)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

async fn phase_memory_kv(
    client: Arc<reqwest::Client>,
    base: &str,
    agent_path_id: &str,
    concurrency: u32,
    kv_ops: u32,
) -> Result<(u32, u32)> {
    let mut ok = 0u32;
    let mut err = 0u32;
    let mut set = JoinSet::new();
    for worker in 0..concurrency {
        let c = Arc::clone(&client);
        let base = base.to_string();
        let aid = agent_path_id.to_string();
        set.spawn(async move {
            let mut local_ok = 0u32;
            let mut local_err = 0u32;
            for i in 0..kv_ops {
                let key = format!("loadtest-kv-{worker}-{i}");
                let url_put = format!("{base}/api/memory/agents/{aid}/kv/{key}");
                let url_get = url_put.clone();
                let body = json!({"value": format!("v-{worker}-{i}")});
                match put_json(&c, &url_put, &body).await {
                    Ok((st, _)) if st.is_success() => local_ok += 1,
                    Ok((st, t)) => {
                        eprintln!("worker {worker} PUT {key} -> {st} {t}");
                        local_err += 1;
                    }
                    Err(e) => {
                        eprintln!("worker {worker} PUT {key}: {e:#}");
                        local_err += 1;
                    }
                }
                match get_json(&c, &url_get).await {
                    Ok(_) => local_ok += 1,
                    Err(e) => {
                        eprintln!("worker {worker} GET {key}: {e:#}");
                        local_err += 1;
                    }
                }
            }
            (local_ok, local_err)
        });
    }
    while let Some(joined) = set.join_next().await {
        let (a, b) = joined.context("kv worker join")?;
        ok += a;
        err += b;
    }
    Ok((ok, err))
}

struct LlmPhaseParams<'a> {
    client: Arc<reqwest::Client>,
    base: &'a str,
    agent_id: &'a str,
    concurrency: u32,
    rounds: u32,
    message: &'a str,
    inter_batch_ms: u64,
    chain_note: Option<&'a str>,
}

async fn phase_llm_messages(p: LlmPhaseParams<'_>) -> Result<(u32, u32)> {
    let rounds = p.rounds;
    let mut ok = 0u32;
    let mut err = 0u32;
    let mut set = JoinSet::new();
    for worker in 0..p.concurrency {
        let c = Arc::clone(&p.client);
        let base = p.base.to_string();
        let aid = p.agent_id.to_string();
        let msg_base = p.message.to_string();
        let chain = p.chain_note.map(|s| s.to_string());
        let stagger = Duration::from_millis(p.inter_batch_ms.saturating_mul(worker as u64));
        set.spawn(async move {
            tokio::time::sleep(stagger).await;
            let mut local_ok = 0u32;
            let mut local_err = 0u32;
            for _r in 0..rounds {
                let mut msg = msg_base.clone();
                if let Some(ref ch) = chain {
                    msg.push_str("\n\n[load-test probe] agent_send chain env: ");
                    msg.push_str(ch);
                }
                let url = format!("{base}/api/agents/{aid}/message");
                let body = json!({ "message": msg });
                match post_json(&c, &url, &body).await {
                    Ok((st, _t)) if st.is_success() => local_ok += 1,
                    Ok((st, t)) => {
                        eprintln!(
                            "worker {worker} message -> {st}: {}",
                            t.chars().take(200).collect::<String>()
                        );
                        local_err += 1;
                    }
                    Err(e) => {
                        eprintln!("worker {worker} message: {e:#}");
                        local_err += 1;
                    }
                }
            }
            (local_ok, local_err)
        });
    }
    while let Some(joined) = set.join_next().await {
        let (a, b) = joined.context("llm worker join")?;
        ok += a;
        err += b;
    }
    Ok((ok, err))
}

async fn phase_workflow_runs(
    client: Arc<reqwest::Client>,
    base: &str,
    agent_id: &str,
    runs: u32,
    max_in_flight: usize,
) -> Result<(u32, u32)> {
    let url_create = format!("{base}/api/workflows");
    let body = json!({
        "name": "xtask-loadtest",
        "description": "Created by cargo xtask load-test",
        "steps": [{
            "name": "step1",
            "agent_id": agent_id,
            "mode": "sequential",
            "prompt": "Reply with the single word OK.",
            "timeout_secs": 120
        }]
    });
    let (st, text) = post_json(&client, &url_create, &body).await?;
    if !st.is_success() {
        bail!("POST /api/workflows -> {st}: {text}");
    }
    let v: serde_json::Value = serde_json::from_str(&text).context("workflow create JSON")?;
    let wf_id = v
        .get("workflow_id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow!("missing workflow_id in response: {text}"))?;

    let sem = Arc::new(Semaphore::new(max_in_flight.max(1)));
    let mut set = JoinSet::new();
    let mut ok = 0u32;
    let mut err = 0u32;
    for i in 0..runs {
        let c = Arc::clone(&client);
        let base = base.to_string();
        let wid = wf_id.to_string();
        let permit = sem.clone().acquire_owned().await.expect("semaphore");
        set.spawn(async move {
            let _p = permit;
            let url = format!("{base}/api/workflows/{wid}/run");
            let body = json!({"input": format!("loadtest run {i}")});
            match post_json(&c, &url, &body).await {
                Ok((st, _)) if st.is_success() => (1u32, 0u32),
                Ok((st, t)) => {
                    eprintln!(
                        "workflow run {i} -> {st}: {}",
                        t.chars().take(120).collect::<String>()
                    );
                    (0u32, 1u32)
                }
                Err(e) => {
                    eprintln!("workflow run {i}: {e:#}");
                    (0u32, 1u32)
                }
            }
        });
    }
    while let Some(joined) = set.join_next().await {
        let (a, b) = joined.context("workflow worker join")?;
        ok += a;
        err += b;
    }
    Ok((ok, err))
}

pub async fn run_load_test(cli: LoadTestCli) -> Result<()> {
    if std::env::var("ARMARAOS_LOAD_TEST").ok().as_deref() != Some("1") {
        bail!(
            "Refusing to run: set ARMARAOS_LOAD_TEST=1 to acknowledge cost/rate-limit risk.\n\
             See docs/load-testing.md."
        );
    }

    let base_url = normalize_base_url(
        &cli.base_url
            .clone()
            .or_else(|| env_trim("ARMARAOS_TEST_BASE_URL"))
            .unwrap_or_else(|| "http://127.0.0.1:4200".into()),
    );

    let agent_id = cli
        .agent_id
        .clone()
        .or_else(|| env_trim("ARMARAOS_TEST_AGENT_ID"))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "ARMARAOS_TEST_AGENT_ID is required (or pass via CLI when wired). \
                 UUID from GET /api/agents."
            )
        })?;

    let bearer = cli
        .bearer
        .clone()
        .or_else(|| env_trim("ARMARAOS_TEST_BEARER"));

    let concurrency = clamp_concurrency(
        cli.concurrency
            .or_else(|| env_u32("ARMARAOS_TEST_CONCURRENCY"))
            .unwrap_or(24),
    );

    let kv_ops = cli
        .kv_ops
        .or_else(|| env_u32("ARMARAOS_TEST_KV_OPS"))
        .unwrap_or(50);

    let message_rounds = cli
        .message_rounds
        .or_else(|| env_u32("ARMARAOS_TEST_MESSAGE_ROUNDS"))
        .unwrap_or(1);

    let message = cli
        .message
        .clone()
        .or_else(|| env_trim("ARMARAOS_TEST_MESSAGE"))
        .unwrap_or_else(|| "loadtest: reply with the single word OK.".into());

    let workflow_runs = cli
        .workflow_runs
        .or_else(|| env_u32("ARMARAOS_TEST_WORKFLOW_RUNS"))
        .unwrap_or(0);

    let inter_batch_ms = cli
        .inter_batch_ms
        .or_else(|| env_u64("ARMARAOS_TEST_INTER_BATCH_MS"))
        .unwrap_or(0);

    let max_wall_secs = cli
        .max_wall_secs
        .or_else(|| env_u64("ARMARAOS_TEST_MAX_WALL_SECS"))
        .unwrap_or(900);

    let config_path = cli
        .config_path
        .clone()
        .or_else(|| env_trim("ARMARAOS_TEST_CONFIG_PATH").map(PathBuf::from))
        .unwrap_or_else(default_config_path);

    let chain_env = env_trim("ARMARAOS_LOAD_TEST_AGENT_SEND_CHAIN");
    let dry_run = cli.dry_run;

    eprintln!("=== ArmaraOS load-test (ARMARAOS_LOAD_TEST=1) ===");
    eprintln!("base_url={base_url}");
    eprintln!("agent_id={agent_id}");
    eprintln!("concurrency={concurrency} kv_ops_per_worker={kv_ops} message_rounds={message_rounds} workflow_runs={workflow_runs}");
    eprintln!("inter_batch_ms(stagger)={inter_batch_ms} max_wall_secs={max_wall_secs}");
    if dry_run {
        eprintln!("mode=DRY-RUN (preflight + metrics only, no KV/LLM/workflow stress)");
    }
    if workflow_runs > 0 {
        eprintln!("warning: workflow_runs={workflow_runs} — each run invokes the workflow step (typically one LLM call per run).");
    }

    let client = Arc::new(build_client(bearer.as_deref())?);
    let isolation = read_driver_isolation(&config_path);
    print_isolation_note(&config_path, isolation.as_deref());

    let inner = async move {
        let t0 = Instant::now();
        get_text(&client, &format!("{base_url}/api/health"))
            .await
            .context("preflight GET /api/health")?;
        let agents = get_json(&client, &format!("{base_url}/api/agents"))
            .await
            .context("preflight GET /api/agents")?;
        if !agent_exists(&agents, &agent_id) {
            bail!("agent id {agent_id} not found in GET /api/agents — pick a Running agent UUID");
        }

        let metrics_before = get_text(&client, &format!("{base_url}/api/metrics")).await?;
        print_llm_metric_lines(&metrics_before);

        if dry_run {
            println!("dry-run: skipping memory / message / workflow phases.");
            println!("wall_ms={}", t0.elapsed().as_millis());
            return Ok(());
        }

        println!("--- phase: memory KV (PUT+GET) ---");
        let kv_path_id = "00000000-0000-0000-0000-000000000001";
        let (kv_ok, kv_err) = phase_memory_kv(
            Arc::clone(&client),
            &base_url,
            kv_path_id,
            concurrency,
            kv_ops,
        )
        .await?;
        println!("memory_kv: ok_ops={kv_ok} err_ops={kv_err}");

        println!("--- phase: LLM (POST /api/agents/.../message) ---");
        let (m_ok, m_err) = phase_llm_messages(LlmPhaseParams {
            client: Arc::clone(&client),
            base: base_url.as_str(),
            agent_id: agent_id.as_str(),
            concurrency,
            rounds: message_rounds,
            message: message.as_str(),
            inter_batch_ms,
            chain_note: chain_env.as_deref(),
        })
        .await?;
        println!("llm_messages: ok={m_ok} err={m_err}");

        if workflow_runs > 0 {
            println!("--- phase: workflow runs ---");
            let max_in_flight = concurrency.clamp(1, 32) as usize;
            let (w_ok, w_err) = phase_workflow_runs(
                Arc::clone(&client),
                &base_url,
                &agent_id,
                workflow_runs,
                max_in_flight,
            )
            .await?;
            println!("workflow_runs: ok={w_ok} err={w_err}");
        }

        let metrics_after = get_text(&client, &format!("{base_url}/api/metrics")).await?;
        print_llm_metric_lines(&metrics_after);

        match get_json(&client, &format!("{base_url}/api/usage/summary")).await {
            Ok(u) => println!(
                "usage/summary: {}",
                u.to_string().chars().take(500).collect::<String>()
            ),
            Err(e) => eprintln!("usage/summary (optional): {e:#}"),
        }

        println!("wall_ms={}", t0.elapsed().as_millis());
        Ok(())
    };

    tokio::time::timeout(Duration::from_secs(max_wall_secs), inner)
        .await
        .map_err(|_| anyhow!("load-test exceeded max_wall_secs={max_wall_secs}"))??;
    Ok(())
}
