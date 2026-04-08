//! Startup garbage collection for internal automation/probe agents.

use openfang_kernel::OpenFangKernel;
use openfang_types::config::KernelConfig;
use openfang_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};

#[tokio::test]
async fn gc_removes_probe_agent_with_no_cron_on_next_boot() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let data_dir = home.join("data");
    std::fs::create_dir_all(&data_dir).expect("data_dir");
    let config = KernelConfig {
        home_dir: home.clone(),
        data_dir,
        ..Default::default()
    };

    {
        let kernel = OpenFangKernel::boot_with_config(config.clone()).expect("boot");
        let assistant = kernel
            .registry
            .list()
            .into_iter()
            .next()
            .expect("assistant");
        let mut manifest = assistant.manifest.clone();
        manifest.name = "allowlist-probe-gc-unref".into();
        kernel.spawn_agent(manifest).expect("spawn probe");
    }

    let kernel2 = OpenFangKernel::boot_with_config(config).expect("boot2");
    let names: Vec<String> = kernel2
        .registry
        .list()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(
        !names.iter().any(|n| n == "allowlist-probe-gc-unref"),
        "orphan probe should be GC'd, got {names:?}"
    );
    assert!(names.iter().any(|n| n == "assistant"));
}

#[tokio::test]
async fn gc_keeps_probe_agent_referenced_by_cron() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let data_dir = home.join("data");
    std::fs::create_dir_all(&data_dir).expect("data_dir");
    let config = KernelConfig {
        home_dir: home.clone(),
        data_dir,
        ..Default::default()
    };

    {
        let kernel = OpenFangKernel::boot_with_config(config.clone()).expect("boot");
        let assistant = kernel
            .registry
            .list()
            .into_iter()
            .next()
            .expect("assistant");
        let mut manifest = assistant.manifest.clone();
        manifest.name = "allowlist-probe-gc-kept".into();
        let probe_id = kernel.spawn_agent(manifest).expect("spawn probe");

        let job = CronJob {
            id: CronJobId::new(),
            agent_id: probe_id,
            name: "gc-test-keep-job".into(),
            enabled: true,
            schedule: CronSchedule::Cron {
                expr: "0 0 * * *".into(),
                tz: None,
            },
            action: CronAction::SystemEvent {
                text: "ping".into(),
            },
            delivery: CronDelivery::None,
            created_at: chrono::Utc::now(),
            last_run: None,
            next_run: None,
        };
        kernel.cron_scheduler.add_job(job, false).expect("add_job");
        kernel.cron_scheduler.persist().expect("persist cron");
    }

    let kernel2 = OpenFangKernel::boot_with_config(config).expect("boot2");
    let names: Vec<String> = kernel2
        .registry
        .list()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(
        names.iter().any(|n| n == "allowlist-probe-gc-kept"),
        "probe with cron should remain, got {names:?}"
    );
}
