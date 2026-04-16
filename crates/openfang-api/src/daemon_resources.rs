//! Background sampling of the daemon process CPU + memory for the dashboard.

use serde::Serialize;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind,
    System,
};

#[derive(Clone, Debug, Serialize)]
pub struct DaemonResourcesSnapshot {
    /// Average CPU usage over the last sample window, normalized to 0–100% of total
    /// machine CPU capacity (all logical cores). See `sysinfo::Process::cpu_usage`.
    pub cpu_percent: f32,
    /// CPU core-equivalent (`cpu_percent / 100 * logical_cpus`), capped at `logical_cpus`.
    pub cpu_cores_equivalent: f32,
    pub logical_cpus: u32,
    /// Resident set size (RSS) for the daemon process (bytes).
    pub memory_rss_bytes: u64,
    /// Installed physical RAM (bytes).
    pub memory_total_bytes: u64,
    /// `memory_rss_bytes / memory_total_bytes * 100` when total is known.
    pub memory_percent: f32,
    pub sampled_at_ms: i64,
    /// `false` when `sysinfo` cannot sample this platform build (e.g. unsupported OS).
    pub supported: bool,
}

pub struct DaemonResources {
    snap: RwLock<DaemonResourcesSnapshot>,
}

impl DaemonResources {
    pub fn spawn_collector() -> Arc<Self> {
        let logical_cpus = std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(1)
            .max(1);

        let initial = DaemonResourcesSnapshot {
            cpu_percent: 0.0,
            cpu_cores_equivalent: 0.0,
            logical_cpus,
            memory_rss_bytes: 0,
            memory_total_bytes: 0,
            memory_percent: 0.0,
            sampled_at_ms: 0,
            supported: sysinfo::IS_SUPPORTED_SYSTEM,
        };

        let this = Arc::new(Self {
            snap: RwLock::new(initial),
        });

        let weak = Arc::downgrade(&this);
        std::thread::Builder::new()
            .name("armaraos-daemon-resources".to_string())
            .spawn(move || collector_main(weak))
            .expect("spawn armaraos daemon resource collector");

        this
    }

    pub fn snapshot(&self) -> DaemonResourcesSnapshot {
        match self.snap.read() {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn publish(&self, next: DaemonResourcesSnapshot) {
        match self.snap.write() {
            Ok(mut g) => *g = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
    }
}

fn logical_cpus_from_system(system: &System) -> u32 {
    let n = system.cpus().len();
    if n > 0 {
        n as u32
    } else {
        std::thread::available_parallelism()
            .map(|c| c.get() as u32)
            .unwrap_or(1)
            .max(1)
    }
}

fn collector_main(state: std::sync::Weak<DaemonResources>) {
    if !sysinfo::IS_SUPPORTED_SYSTEM {
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let Some(res) = state.upgrade() else {
                return;
            };
            let logical_cpus = std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(1)
                .max(1);
            res.publish(DaemonResourcesSnapshot {
                cpu_percent: 0.0,
                cpu_cores_equivalent: 0.0,
                logical_cpus,
                memory_rss_bytes: 0,
                memory_total_bytes: 0,
                memory_percent: 0.0,
                sampled_at_ms: chrono::Utc::now().timestamp_millis(),
                supported: false,
            });
        }
    }

    let pid_u32 = std::process::id();
    let pid = Pid::from_u32(pid_u32);

    let mut system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );

    // Prime CPU + process accounting (first readings are often zero).
    system.refresh_cpu_usage();
    system.refresh_memory();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

    loop {
        std::thread::sleep(Duration::from_millis(1200));
        let Some(res) = state.upgrade() else {
            return;
        };

        system.refresh_cpu_usage();
        system.refresh_memory();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            false,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );

        let logical_cpus = logical_cpus_from_system(&system);
        let logical_f = logical_cpus as f32;

        let total_mem = system.total_memory();
        let proc = system.process(pid);
        let rss = proc.map(|p| p.memory()).unwrap_or(0);
        let cpu_raw = proc.map(|p| p.cpu_usage()).unwrap_or(0.0);

        // sysinfo: process CPU usage may exceed 100 on multi-core machines; normalize to
        // a 0–100% "share of total CPU capacity" for the UI.
        let cpu_machine_pct = (cpu_raw / logical_f).clamp(0.0, 100.0);
        let core_equiv = (cpu_raw / 100.0).clamp(0.0, logical_f);

        let mem_pct = if total_mem > 0 {
            ((rss as f64 / total_mem as f64) * 100.0) as f32
        } else {
            0.0
        };

        res.publish(DaemonResourcesSnapshot {
            cpu_percent: cpu_machine_pct,
            cpu_cores_equivalent: core_equiv,
            logical_cpus,
            memory_rss_bytes: rss,
            memory_total_bytes: total_mem,
            memory_percent: mem_pct.clamp(0.0, 100.0),
            sampled_at_ms: chrono::Utc::now().timestamp_millis(),
            supported: true,
        });
    }
}
