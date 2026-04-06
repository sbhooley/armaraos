//! `CronAction::AinlRun` execution with a stub `ainl` binary (Unix).

#![cfg(unix)]

use openfang_kernel::OpenFangKernel;
use openfang_types::config::KernelConfig;
use openfang_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
use serial_test::serial;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

#[tokio::test]
#[serial]
async fn cron_run_job_ainl_run_executes_stub_binary() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let lib = home.join("ainl-library");
    std::fs::create_dir_all(&lib).expect("create ainl-library");

    let prog = lib.join("stub.ainl");
    std::fs::write(&prog, "S app core noop\nL1:\n R core.ADD 1 1 ->x\n J x\n").expect("write prog");

    let fake_ainl = home.join("fake-ainl");
    let mut f = std::fs::File::create(&fake_ainl).expect("fake file");
    f.write_all(
        b"#!/bin/sh\nif [ \"$1\" != \"run\" ]; then exit 1; fi\nshift\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    --json) shift ;;\n    --frame-json) shift; shift ;;\n    *) break ;;\n  esac\ndone\n# remaining arg is .ainl path\necho '{\"ok\":true,\"label\":\"test\",\"result\":42,\"runtime_version\":\"stub\"}'\nexit 0\n",
    )
    .expect("script");
    drop(f);
    std::fs::set_permissions(&fake_ainl, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    unsafe {
        std::env::set_var("ARMARAOS_AINL_BIN", fake_ainl.to_str().unwrap());
    }

    let config = KernelConfig {
        home_dir: home.clone(),
        ..Default::default()
    };
    let kernel = OpenFangKernel::boot_with_config(config).expect("boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let assistant = kernel
        .registry
        .list()
        .into_iter()
        .next()
        .expect("at least one agent after boot");

    let job = CronJob {
        id: CronJobId::new(),
        agent_id: assistant.id,
        name: "test-ainl-stub".into(),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: "0 0 * * *".into(),
            tz: None,
        },
        action: CronAction::AinlRun {
            program_path: "stub.ainl".into(),
            cwd: None,
            ainl_binary: None,
            timeout_secs: Some(30),
            json_output: true,
            frame: None,
        },
        delivery: CronDelivery::None,
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    };

    let out = kernel.cron_run_job(&job).await.expect("cron_run_job");
    assert!(
        out.contains("\"ok\": true") || out.contains("\"ok\":true"),
        "{}",
        out
    );

    unsafe {
        std::env::remove_var("ARMARAOS_AINL_BIN");
    }
}

#[tokio::test]
#[serial]
async fn cron_run_job_ainl_run_passes_frame_json() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let lib = home.join("ainl-library");
    std::fs::create_dir_all(&lib).expect("create ainl-library");

    let prog = lib.join("stub.ainl");
    std::fs::write(&prog, "stub").expect("write prog");

    let fake_ainl = home.join("fake-ainl");
    let mut f = std::fs::File::create(&fake_ainl).expect("fake file");
    f.write_all(
        b"#!/bin/sh\nif [ \"$1\" != \"run\" ]; then exit 1; fi\nshift\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    --json) shift ;;\n    --frame-json) shift; shift ;;\n    *) break ;;\n  esac\ndone\necho '{\"ok\":true,\"label\":\"test\",\"result\":42,\"runtime_version\":\"stub\"}'\nexit 0\n",
    )
    .expect("script");
    drop(f);
    std::fs::set_permissions(&fake_ainl, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    unsafe {
        std::env::set_var("ARMARAOS_AINL_BIN", fake_ainl.to_str().unwrap());
    }

    let config = KernelConfig {
        home_dir: home.clone(),
        ..Default::default()
    };
    let kernel = OpenFangKernel::boot_with_config(config).expect("boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let assistant = kernel
        .registry
        .list()
        .into_iter()
        .next()
        .expect("at least one agent after boot");

    let job = CronJob {
        id: CronJobId::new(),
        agent_id: assistant.id,
        name: "test-ainl-frame".into(),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: "0 0 * * *".into(),
            tz: None,
        },
        action: CronAction::AinlRun {
            program_path: "stub.ainl".into(),
            cwd: None,
            ainl_binary: None,
            timeout_secs: Some(30),
            json_output: true,
            frame: Some(serde_json::json!({
                "frame_version": "1",
                "op": "skill_mint",
                "run_id": "cron-test",
                "intent": "test",
                "outcome": "ok",
                "episode": "e",
                "tier": "assisted"
            })),
        },
        delivery: CronDelivery::None,
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    };

    let out = kernel.cron_run_job(&job).await.expect("cron_run_job");
    assert!(
        out.contains("\"ok\": true") || out.contains("\"ok\":true"),
        "{}",
        out
    );

    unsafe {
        std::env::remove_var("ARMARAOS_AINL_BIN");
    }
}

/// Scheduled `ainl run` sets `AINL_HOST_ADAPTER_ALLOWLIST` from agent manifest metadata
/// (`ainl_host_adapter_allowlist`) so subprocess policy matches the kernel-derived grant.
#[tokio::test]
#[serial]
async fn cron_run_job_ainl_run_sets_host_adapter_allowlist_from_agent_metadata() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let lib = home.join("ainl-library");
    std::fs::create_dir_all(&lib).expect("create ainl-library");

    let prog = lib.join("stub.ainl");
    std::fs::write(&prog, "S app core noop\nL1:\n R core.ADD 1 1 ->x\n J x\n").expect("write prog");

    let probe = tmp.path().join("allowlist_probe.txt");

    let fake_ainl = home.join("fake-ainl");
    let mut f = std::fs::File::create(&fake_ainl).expect("fake file");
    f.write_all(
        br#"#!/bin/sh
if [ "$1" != "run" ]; then exit 1; fi
shift
while [ $# -gt 0 ]; do
  case "$1" in
    --json) shift ;;
    --frame-json) shift; shift ;;
    *) break ;;
  esac
done
if [ -n "$ARMARAOS_TEST_ALLOWLIST_OUT" ]; then
  printf '%s' "$AINL_HOST_ADAPTER_ALLOWLIST" > "$ARMARAOS_TEST_ALLOWLIST_OUT"
fi
echo '{"ok":true,"label":"test","result":42,"runtime_version":"stub"}'
exit 0
"#,
    )
    .expect("script");
    drop(f);
    std::fs::set_permissions(&fake_ainl, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    unsafe {
        std::env::set_var("ARMARAOS_AINL_BIN", fake_ainl.to_str().unwrap());
        std::env::set_var(
            "ARMARAOS_TEST_ALLOWLIST_OUT",
            probe.to_str().expect("utf8 path"),
        );
    }

    let config = KernelConfig {
        home_dir: home.clone(),
        ..Default::default()
    };
    let kernel = OpenFangKernel::boot_with_config(config).expect("boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let assistant = kernel
        .registry
        .list()
        .into_iter()
        .next()
        .expect("at least one agent after boot");

    let mut manifest = assistant.manifest.clone();
    manifest.name = format!(
        "allowlist-probe-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    manifest.metadata.insert(
        "ainl_host_adapter_allowlist".to_string(),
        serde_json::json!("http,llm"),
    );

    let agent_id = kernel.spawn_agent(manifest).expect("spawn probe agent");

    let job = CronJob {
        id: CronJobId::new(),
        agent_id,
        name: "test-ainl-allowlist-env".into(),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: "0 0 * * *".into(),
            tz: None,
        },
        action: CronAction::AinlRun {
            program_path: "stub.ainl".into(),
            cwd: None,
            ainl_binary: None,
            timeout_secs: Some(30),
            json_output: true,
            frame: None,
        },
        delivery: CronDelivery::None,
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    };

    let out = kernel.cron_run_job(&job).await.expect("cron_run_job");
    assert!(
        out.contains("\"ok\": true") || out.contains("\"ok\":true"),
        "{}",
        out
    );

    let seen = std::fs::read_to_string(&probe).expect("probe file");
    assert_eq!(seen, "http,llm");

    unsafe {
        std::env::remove_var("ARMARAOS_AINL_BIN");
        std::env::remove_var("ARMARAOS_TEST_ALLOWLIST_OUT");
    }
}

/// Offline-style agents do not get `AINL_HOST_ADAPTER_ALLOWLIST` from the kernel; the subprocess
/// must not inherit a narrow value from the parent process (e.g. daemon env).
#[tokio::test]
#[serial]
async fn cron_run_job_ainl_run_clears_inherited_host_adapter_allowlist_for_offline_agent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let lib = home.join("ainl-library");
    std::fs::create_dir_all(&lib).expect("create ainl-library");

    let prog = lib.join("stub.ainl");
    std::fs::write(&prog, "S app core noop\nL1:\n R core.ADD 1 1 ->x\n J x\n").expect("write prog");

    let probe = tmp.path().join("allowlist_probe_offline.txt");

    let fake_ainl = home.join("fake-ainl");
    let mut f = std::fs::File::create(&fake_ainl).expect("fake file");
    f.write_all(
        br#"#!/bin/sh
if [ "$1" != "run" ]; then exit 1; fi
shift
while [ $# -gt 0 ]; do
  case "$1" in
    --json) shift ;;
    --frame-json) shift; shift ;;
    *) break ;;
  esac
done
if [ -n "$ARMARAOS_TEST_ALLOWLIST_OUT" ]; then
  printf '%s' "${AINL_HOST_ADAPTER_ALLOWLIST-__UNSET__}" > "$ARMARAOS_TEST_ALLOWLIST_OUT"
fi
echo '{"ok":true,"label":"test","result":42,"runtime_version":"stub"}'
exit 0
"#,
    )
    .expect("script");
    drop(f);
    std::fs::set_permissions(&fake_ainl, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    unsafe {
        std::env::set_var("ARMARAOS_AINL_BIN", fake_ainl.to_str().unwrap());
        std::env::set_var(
            "ARMARAOS_TEST_ALLOWLIST_OUT",
            probe.to_str().expect("utf8 path"),
        );
        // Simulate a daemon or shell that exported a restrictive allowlist.
        std::env::set_var("AINL_HOST_ADAPTER_ALLOWLIST", "core,http,llm");
    }

    let config = KernelConfig {
        home_dir: home.clone(),
        ..Default::default()
    };
    let kernel = OpenFangKernel::boot_with_config(config).expect("boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    // Default capabilities: no network/tools/shell/OFP → kernel does not set allowlist env.
    let agent_id = kernel
        .spawn_agent(openfang_types::agent::AgentManifest {
            name: format!(
                "offline-cron-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ),
            ..Default::default()
        })
        .expect("spawn offline agent");

    let job = CronJob {
        id: CronJobId::new(),
        agent_id,
        name: "test-ainl-allowlist-clear".into(),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: "0 0 * * *".into(),
            tz: None,
        },
        action: CronAction::AinlRun {
            program_path: "stub.ainl".into(),
            cwd: None,
            ainl_binary: None,
            timeout_secs: Some(30),
            json_output: true,
            frame: None,
        },
        delivery: CronDelivery::None,
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    };

    let out = kernel.cron_run_job(&job).await.expect("cron_run_job");
    assert!(
        out.contains("\"ok\": true") || out.contains("\"ok\":true"),
        "{}",
        out
    );

    let seen = std::fs::read_to_string(&probe).expect("probe file");
    assert_eq!(
        seen, "__UNSET__",
        "child should not inherit AINL_HOST_ADAPTER_ALLOWLIST when kernel omits it"
    );

    unsafe {
        std::env::remove_var("ARMARAOS_AINL_BIN");
        std::env::remove_var("ARMARAOS_TEST_ALLOWLIST_OUT");
        std::env::remove_var("AINL_HOST_ADAPTER_ALLOWLIST");
    }
}

/// Scheduled `ainl run` sets `AINL_ALLOW_IR_DECLARED_ADAPTERS=1` for mass-market defaults.
#[tokio::test]
#[serial]
async fn cron_run_job_ainl_run_sets_allow_ir_declared_adapters_default() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let lib = home.join("ainl-library");
    std::fs::create_dir_all(&lib).expect("create ainl-library");

    let prog = lib.join("stub.ainl");
    std::fs::write(&prog, "S app core noop\nL1:\n R core.ADD 1 1 ->x\n J x\n").expect("write prog");

    let probe = tmp.path().join("allow_ir_probe.txt");

    let fake_ainl = home.join("fake-ainl");
    let mut f = std::fs::File::create(&fake_ainl).expect("fake file");
    f.write_all(
        br#"#!/bin/sh
if [ "$1" != "run" ]; then exit 1; fi
shift
while [ $# -gt 0 ]; do
  case "$1" in
    --json) shift ;;
    --frame-json) shift; shift ;;
    *) break ;;
  esac
done
if [ -n "$ARMARAOS_TEST_ALLOW_IR_OUT" ]; then
  printf '%s' "${AINL_ALLOW_IR_DECLARED_ADAPTERS-__UNSET__}" > "$ARMARAOS_TEST_ALLOW_IR_OUT"
fi
echo '{"ok":true,"label":"test","result":42,"runtime_version":"stub"}'
exit 0
"#,
    )
    .expect("script");
    drop(f);
    std::fs::set_permissions(&fake_ainl, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    unsafe {
        std::env::set_var("ARMARAOS_AINL_BIN", fake_ainl.to_str().unwrap());
        std::env::set_var(
            "ARMARAOS_TEST_ALLOW_IR_OUT",
            probe.to_str().expect("utf8 path"),
        );
    }

    let config = KernelConfig {
        home_dir: home.clone(),
        ..Default::default()
    };
    let kernel = OpenFangKernel::boot_with_config(config).expect("boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let assistant = kernel
        .registry
        .list()
        .into_iter()
        .next()
        .expect("at least one agent after boot");

    let job = CronJob {
        id: CronJobId::new(),
        agent_id: assistant.id,
        name: "test-ainl-allow-ir".into(),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: "0 0 * * *".into(),
            tz: None,
        },
        action: CronAction::AinlRun {
            program_path: "stub.ainl".into(),
            cwd: None,
            ainl_binary: None,
            timeout_secs: Some(30),
            json_output: true,
            frame: None,
        },
        delivery: CronDelivery::None,
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    };

    let out = kernel.cron_run_job(&job).await.expect("cron_run_job");
    assert!(
        out.contains("\"ok\": true") || out.contains("\"ok\":true"),
        "{}",
        out
    );

    let seen = std::fs::read_to_string(&probe).expect("probe file");
    assert_eq!(seen, "1");

    unsafe {
        std::env::remove_var("ARMARAOS_AINL_BIN");
        std::env::remove_var("ARMARAOS_TEST_ALLOW_IR_OUT");
    }
}

/// Manifest `ainl_allow_ir_declared_adapters: "0"` forces subprocess `AINL_ALLOW_IR_DECLARED_ADAPTERS=0`.
#[tokio::test]
#[serial]
async fn cron_run_job_ainl_run_allow_ir_declared_adapters_manifest_off() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let lib = home.join("ainl-library");
    std::fs::create_dir_all(&lib).expect("create ainl-library");

    let prog = lib.join("stub.ainl");
    std::fs::write(&prog, "S app core noop\nL1:\n R core.ADD 1 1 ->x\n J x\n").expect("write prog");

    let probe = tmp.path().join("allow_ir_probe_off.txt");

    let fake_ainl = home.join("fake-ainl");
    let mut f = std::fs::File::create(&fake_ainl).expect("fake file");
    f.write_all(
        br#"#!/bin/sh
if [ "$1" != "run" ]; then exit 1; fi
shift
while [ $# -gt 0 ]; do
  case "$1" in
    --json) shift ;;
    --frame-json) shift; shift ;;
    *) break ;;
  esac
done
if [ -n "$ARMARAOS_TEST_ALLOW_IR_OUT" ]; then
  printf '%s' "${AINL_ALLOW_IR_DECLARED_ADAPTERS-__UNSET__}" > "$ARMARAOS_TEST_ALLOW_IR_OUT"
fi
echo '{"ok":true,"label":"test","result":42,"runtime_version":"stub"}'
exit 0
"#,
    )
    .expect("script");
    drop(f);
    std::fs::set_permissions(&fake_ainl, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    unsafe {
        std::env::set_var("ARMARAOS_AINL_BIN", fake_ainl.to_str().unwrap());
        std::env::set_var(
            "ARMARAOS_TEST_ALLOW_IR_OUT",
            probe.to_str().expect("utf8 path"),
        );
    }

    let config = KernelConfig {
        home_dir: home.clone(),
        ..Default::default()
    };
    let kernel = OpenFangKernel::boot_with_config(config).expect("boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let assistant = kernel
        .registry
        .list()
        .into_iter()
        .next()
        .expect("at least one agent after boot");

    let mut manifest = assistant.manifest.clone();
    manifest.name = format!(
        "allow-ir-off-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    manifest.metadata.insert(
        "ainl_allow_ir_declared_adapters".to_string(),
        serde_json::json!("0"),
    );

    let agent_id = kernel.spawn_agent(manifest).expect("spawn probe agent");

    let job = CronJob {
        id: CronJobId::new(),
        agent_id,
        name: "test-ainl-allow-ir-off".into(),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: "0 0 * * *".into(),
            tz: None,
        },
        action: CronAction::AinlRun {
            program_path: "stub.ainl".into(),
            cwd: None,
            ainl_binary: None,
            timeout_secs: Some(30),
            json_output: true,
            frame: None,
        },
        delivery: CronDelivery::None,
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    };

    let out = kernel.cron_run_job(&job).await.expect("cron_run_job");
    assert!(
        out.contains("\"ok\": true") || out.contains("\"ok\":true"),
        "{}",
        out
    );

    let seen = std::fs::read_to_string(&probe).expect("probe file");
    assert_eq!(seen, "0");

    unsafe {
        std::env::remove_var("ARMARAOS_AINL_BIN");
        std::env::remove_var("ARMARAOS_TEST_ALLOW_IR_OUT");
    }
}
