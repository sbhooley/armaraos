//! `CronAction::AinlRun` execution with a stub `ainl` binary (Unix).

#![cfg(unix)]

use openfang_kernel::OpenFangKernel;
use openfang_types::config::KernelConfig;
use openfang_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

#[tokio::test]
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
