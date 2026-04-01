//! Serializes on `HOME` / `USERPROFILE` so parallel tests do not race on process environment.

use openfang_types::config::{ensure_armaraos_data_home, openfang_home_dir, ArmaraosHomeSetup};
use std::sync::Mutex;

static HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

struct HomeEnvGuard {
    prev_home: Option<String>,
    /// Windows `dirs::home_dir` reads `USERPROFILE`.
    #[cfg_attr(not(windows), allow(dead_code))]
    prev_userprofile: Option<String>,
    prev_arm: Option<String>,
    prev_of: Option<String>,
}

impl HomeEnvGuard {
    fn set(tmp: &std::path::Path) -> Self {
        let prev_home = std::env::var("HOME").ok();
        let prev_userprofile = std::env::var("USERPROFILE").ok();
        let prev_arm = std::env::var("ARMARAOS_HOME").ok();
        let prev_of = std::env::var("OPENFANG_HOME").ok();

        std::env::set_var("HOME", tmp);
        #[cfg(windows)]
        std::env::set_var("USERPROFILE", tmp);
        std::env::remove_var("ARMARAOS_HOME");
        std::env::remove_var("OPENFANG_HOME");

        Self {
            prev_home,
            prev_userprofile,
            prev_arm,
            prev_of,
        }
    }
}

impl Drop for HomeEnvGuard {
    fn drop(&mut self) {
        restore("HOME", self.prev_home.as_deref());
        #[cfg(windows)]
        restore("USERPROFILE", self.prev_userprofile.as_deref());
        restore("ARMARAOS_HOME", self.prev_arm.as_deref());
        restore("OPENFANG_HOME", self.prev_of.as_deref());
    }
}

fn restore(key: &str, val: Option<&str>) {
    match val {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}

#[test]
fn migrates_openfang_dir_to_armaraos() {
    let _lock = HOME_ENV_LOCK.lock().expect("home env lock");
    let tmp = tempfile::tempdir().expect("tempdir");
    let _guard = HomeEnvGuard::set(tmp.path());

    std::fs::create_dir_all(tmp.path().join(".openfang")).unwrap();
    assert_eq!(
        ensure_armaraos_data_home().unwrap(),
        ArmaraosHomeSetup::MigratedFromOpenfang
    );
    assert!(tmp.path().join(".armaraos").is_dir());
    assert!(!tmp.path().join(".openfang").exists());
    assert_eq!(openfang_home_dir(), tmp.path().join(".armaraos"));
}

#[test]
fn creates_armaraos_when_no_legacy_dir() {
    let _lock = HOME_ENV_LOCK.lock().expect("home env lock");
    let tmp = tempfile::tempdir().expect("tempdir");
    let _guard = HomeEnvGuard::set(tmp.path());

    assert_eq!(
        ensure_armaraos_data_home().unwrap(),
        ArmaraosHomeSetup::Created
    );
    assert!(tmp.path().join(".armaraos").is_dir());
    assert_eq!(openfang_home_dir(), tmp.path().join(".armaraos"));
}
