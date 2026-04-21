//! First-launch download of local Whisper + Piper assets into `~/.armaraos/voice/`.
//!
//! Windows: ships `whisper-cli.exe` + DLLs from `whisper-bin-x64.zip`, and `ffmpeg.exe` from the
//! Gyan FFmpeg essentials build (extracted to `voice/ffmpeg_win/bin/`) when missing.
//! macOS: Homebrew at `/opt/homebrew` or `/usr/local` runs `brew install whisper-cpp` and
//! `brew install ffmpeg` when those binaries are missing (WebM→WAV transcoding for whisper-cli).
//! Skipped in CI or when `ARMARAOS_SKIP_BREW_*` env vars are set.
//! Linux: no bundled ffmpeg download yet — use distro packages or set `FFMPEG_PATH`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use tracing::{info, warn};

#[cfg(windows)]
const WHISPER_CPP_RELEASE_TAG: &str = "v1.8.4";
const PIPER_TAG: &str = "2023.11.14-2";

const HF_GGML_BASE: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin";
const HF_PIPER_VOICE: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx";
const HF_PIPER_JSON: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json";

/// Populate `[local_voice]` paths by downloading binaries/models when unset.
///
/// Mutates `cfg` in memory only (does not rewrite `config.toml`). Idempotent when files already
/// exist under `home_dir/voice/`.
///
/// Runs the heavy work on a dedicated OS thread: `reqwest::blocking` must not execute on a Tokio
/// worker (e.g. `#[tokio::test]` calling `OpenFangKernel::boot_with_config`), or Tokio panics when
/// the nested blocking runtime tears down.
pub fn ensure_local_voice(home_dir: &Path, cfg: &mut openfang_types::config::LocalVoiceConfig) {
    if !cfg.enabled || !cfg.auto_download {
        return;
    }

    let home = home_dir.to_path_buf();
    let mut inner = cfg.clone();
    match std::thread::Builder::new()
        .name("openfang-local-voice".into())
        .spawn(move || {
            ensure_local_voice_impl(&home, &mut inner);
            inner
        }) {
        Ok(handle) => match handle.join() {
            Ok(out) => *cfg = out,
            Err(_) => warn!("local_voice: bootstrap thread panicked"),
        },
        Err(e) => warn!(error = %e, "local_voice: failed to spawn bootstrap thread"),
    }
}

fn ensure_local_voice_impl(home_dir: &Path, cfg: &mut openfang_types::config::LocalVoiceConfig) {
    let voice_root = home_dir.join("voice");
    if let Err(e) = std::fs::create_dir_all(&voice_root) {
        warn!(error = %e, "local_voice: failed to create voice directory");
        return;
    }

    let client = match Client::builder()
        .user_agent(crate::USER_AGENT)
        .timeout(Duration::from_secs(900))
        .connect_timeout(Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "local_voice: failed to build HTTP client");
            return;
        }
    };

    if let Err(e) = run_bootstrap(&client, &voice_root, cfg) {
        warn!(error = %e, "local_voice auto-download incomplete");
    }
}

fn run_bootstrap(
    client: &Client,
    voice_root: &Path,
    cfg: &mut openfang_types::config::LocalVoiceConfig,
) -> Result<(), String> {
    let models_dir = voice_root.join("models");
    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("create models dir: {e}"))?;

    if cfg.whisper_model.is_none() {
        let model_path = models_dir.join("ggml-base.bin");
        if !model_path.is_file() || file_len(&model_path)? < 50_000_000 {
            info!("local_voice: downloading Whisper ggml-base model (~150 MiB)…");
            download_to_file(
                client,
                HF_GGML_BASE,
                &model_path,
                "application/octet-stream",
            )?;
        }
        cfg.whisper_model = Some(model_path);
    }

        if cfg.whisper_cli.is_none() {
            #[cfg(windows)]
            {
                ensure_whisper_windows(client, voice_root, cfg)?;
            }
            #[cfg(not(windows))]
            {
                let mut found = probe_whisper_cli_unix();
                #[cfg(target_os = "macos")]
                if found.is_none() {
                    match brew_install_whisper_cpp_macos() {
                        Ok(()) => {
                            found = probe_whisper_cli_unix();
                            if found.is_some() {
                                info!("local_voice: whisper-cli available after Homebrew install");
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "local_voice: automatic `brew install whisper-cpp` skipped or failed");
                        }
                    }
                }
                if let Some(p) = found {
                    info!(path = %p.display(), "local_voice: using whisper-cli (Homebrew cellar or PATH)");
                    cfg.whisper_cli = Some(p);
                } else {
                    warn!(
                        "local_voice: whisper-cli not found. Install `whisper-cpp` (e.g. `brew install whisper-cpp` on macOS or your distro package) \
                         or set [local_voice] whisper_cli in config.toml. The ggml-base.bin model is auto-downloaded to ~/.armaraos/voice/models/ when missing."
                    );
                }
            }
        }

    #[cfg(windows)]
    {
        if !probe_ffmpeg_windows(voice_root) {
            if let Err(e) = ensure_ffmpeg_windows(client, voice_root) {
                warn!(error = %e, "local_voice: automatic Windows ffmpeg download failed");
            } else {
                info!("local_voice: ffmpeg.exe ready under voice/ffmpeg_win/");
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if !probe_ffmpeg_unix() {
            match brew_install_ffmpeg_macos() {
                Ok(()) => info!("local_voice: ffmpeg available (WebM voice → WAV for whisper)"),
                Err(e) => warn!(error = %e, "local_voice: automatic `brew install ffmpeg` skipped or failed"),
            }
        }
    }

    let piper_root = voice_root.join("piper_bundle");
    if cfg.piper_binary.is_none() {
        match ensure_piper_bundle(client, voice_root, &piper_root) {
            Ok(()) => {
                let exe = piper_executable_path(&piper_root);
                if exe.is_file() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Ok(meta) = std::fs::metadata(&exe) {
                            let mut perms = meta.permissions();
                            perms.set_mode(0o755);
                            let _ = std::fs::set_permissions(&exe, perms);
                        }
                    }
                    cfg.piper_binary = Some(exe);
                }
            }
            Err(e) => warn!(error = %e, "local_voice: Piper bundle skipped"),
        }
    }

    if cfg.piper_voice.is_none() {
        let voices_dir = voice_root.join("voices");
        std::fs::create_dir_all(&voices_dir).map_err(|e| format!("voices dir: {e}"))?;
        let onnx = voices_dir.join("en_US-lessac-medium.onnx");
        let json = voices_dir.join("en_US-lessac-medium.onnx.json");
        if !onnx.is_file() {
            info!("local_voice: downloading Piper voice (en_US lessac medium)…");
            download_to_file(client, HF_PIPER_VOICE, &onnx, "application/octet-stream")?;
        }
        if !json.is_file() {
            let _ = download_to_file(client, HF_PIPER_JSON, &json, "application/json");
        }
        cfg.piper_voice = Some(onnx);
    }

    Ok(())
}

fn piper_executable_path(piper_root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        piper_root.join("piper").join("piper.exe")
    }
    #[cfg(not(windows))]
    {
        piper_root.join("piper").join("piper")
    }
}

fn file_len(p: &Path) -> Result<u64, String> {
    Ok(std::fs::metadata(p)
        .map_err(|e| format!("metadata: {e}"))?
        .len())
}

fn download_to_file(
    client: &Client,
    url: &str,
    dest: &Path,
    _kind: &str,
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let tmp = dest.with_extension("partial");
    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GET {url} -> HTTP {}", resp.status()));
    }
    let mut f = std::fs::File::create(&tmp).map_err(|e| format!("create temp: {e}"))?;
    std::io::copy(&mut resp, &mut f).map_err(|e| format!("read body {url}: {e}"))?;
    f.flush().ok();
    drop(f);
    std::fs::rename(&tmp, dest).map_err(|e| format!("rename temp: {e}"))?;
    Ok(())
}

#[cfg(windows)]
fn ensure_whisper_windows(
    client: &Client,
    voice_root: &Path,
    cfg: &mut openfang_types::config::LocalVoiceConfig,
) -> Result<(), String> {
    let dest_dir = voice_root.join("whisper_cpp_win");
    let exe = dest_dir.join("whisper-cli.exe");
    if !exe.is_file() {
        let url = format!(
            "https://github.com/ggml-org/whisper.cpp/releases/download/{WHISPER_CPP_RELEASE_TAG}/whisper-bin-x64.zip"
        );
        info!("local_voice: downloading whisper.cpp CLI for Windows…");
        let bytes = client
            .get(&url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("GET {url}: {e}"))?
            .bytes()
            .map_err(|e| format!("body: {e}"))?;
        std::fs::create_dir_all(&dest_dir).map_err(|e| format!("mkdir whisper: {e}"))?;
        let cursor = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("zip: {e}"))?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).map_err(|e| format!("zip index: {e}"))?;
            let name = file.name().to_string();
            let Some(rest) = name.strip_prefix("Release/") else {
                continue;
            };
            let out_path = dest_dir.join(rest);
            if name.ends_with('/') {
                std::fs::create_dir_all(&out_path).ok();
                continue;
            }
            if let Some(p) = out_path.parent() {
                std::fs::create_dir_all(p).map_err(|e| format!("mkdir {:?}: {e}", p))?;
            }
            let mut out = std::fs::File::create(&out_path).map_err(|e| format!("create file: {e}"))?;
            std::io::copy(&mut file, &mut out).map_err(|e| format!("write {:?}: {e}", out_path))?;
        }
    }
    if exe.is_file() {
        cfg.whisper_cli = Some(exe);
    }
    Ok(())
}

/// Gyan FFmpeg essentials (win64): `bin/ffmpeg.exe` under a `*-essentials_build` folder. Redirects to a versioned zip.
#[cfg(windows)]
const FFMPEG_WIN_ESSENTIALS_ZIP: &str =
    "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";

#[cfg(windows)]
fn probe_ffmpeg_windows(voice_root: &Path) -> bool {
    let local = voice_root.join("ffmpeg_win/bin/ffmpeg.exe");
    if local.is_file() {
        return true;
    }
    for p in [
        PathBuf::from(r"C:\ffmpeg\bin\ffmpeg.exe"),
        PathBuf::from(r"C:\Program Files\ffmpeg\bin\ffmpeg.exe"),
    ] {
        if p.is_file() {
            return true;
        }
    }
    let out = std::process::Command::new("where")
        .arg("ffmpeg")
        .output()
        .ok();
    out.map(|o| o.status.success()).unwrap_or(false)
}

#[cfg(windows)]
fn ensure_ffmpeg_windows(client: &Client, voice_root: &Path) -> Result<(), String> {
    let bin_dir = voice_root.join("ffmpeg_win/bin");
    let exe = bin_dir.join("ffmpeg.exe");
    if exe.is_file() {
        return Ok(());
    }
    info!("local_voice: downloading FFmpeg for Windows (essentials build)…");
    let bytes = client
        .get(FFMPEG_WIN_ESSENTIALS_ZIP)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("GET ffmpeg zip: {e}"))?
        .bytes()
        .map_err(|e| format!("body: {e}"))?;
    std::fs::create_dir_all(&bin_dir).map_err(|e| format!("mkdir ffmpeg_win: {e}"))?;
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("ffmpeg zip: {e}"))?;
    let mut got = false;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| format!("zip idx: {e}"))?;
        let name = file.name().replace('\\', "/");
        if !name.to_lowercase().ends_with("/bin/ffmpeg.exe") {
            continue;
        }
        let mut out = std::fs::File::create(&exe).map_err(|e| format!("create ffmpeg.exe: {e}"))?;
        std::io::copy(&mut file, &mut out).map_err(|e| format!("write ffmpeg: {e}"))?;
        got = true;
        break;
    }
    if !got {
        return Err(
            "ffmpeg-release-essentials.zip did not contain bin/ffmpeg.exe (upstream layout changed?)"
                .into(),
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn probe_whisper_cli_unix() -> Option<PathBuf> {
    // Typical Homebrew installs (interactive shells put these on PATH; the daemon may not inherit PATH).
    let candidates = [
        "/opt/homebrew/opt/whisper-cpp/bin/whisper-cli",
        "/usr/local/opt/whisper-cpp/bin/whisper-cli",
        "/opt/homebrew/bin/whisper-cli",
        "/usr/local/bin/whisper-cli",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(p) = probe_whisper_via_homebrew_prefix() {
        return Some(p);
    }
    let out = std::process::Command::new("sh")
        .arg("-lc")
        .arg("command -v whisper-cli 2>/dev/null || true")
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return None;
    }
    let p = PathBuf::from(s);
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

/// Typical Apple Silicon / Intel Homebrew locations (may not be on `PATH` for daemons).
#[cfg(not(windows))]
fn homebrew_executable() -> Option<PathBuf> {
    ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

/// On macOS, install `whisper-cpp` via Homebrew when it is missing (idempotent).
#[cfg(all(not(windows), target_os = "macos"))]
fn brew_install_whisper_cpp_macos() -> Result<(), String> {
    if std::env::var("ARMARAOS_SKIP_BREW_WHISPER")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return Err("ARMARAOS_SKIP_BREW_WHISPER is set".into());
    }
    if matches!(
        std::env::var("CI").ok().as_deref(),
        Some("true") | Some("1") | Some("True")
    ) {
        return Err("CI is set".into());
    }
    let Some(brew) = homebrew_executable() else {
        return Err("Homebrew not found (/opt/homebrew/bin/brew or /usr/local/bin/brew)".into());
    };
    info!(
        "local_voice: installing `whisper-cpp` via Homebrew (first-time setup; may take a few minutes)…"
    );
    let status = std::process::Command::new(&brew)
        .args(["install", "whisper-cpp"])
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("NONINTERACTIVE", "1")
        .status()
        .map_err(|e| format!("failed to spawn brew: {e}"))?;
    if !status.success() {
        return Err(format!("brew install whisper-cpp exited with {status}"));
    }
    Ok(())
}

/// True when `ffmpeg` exists on PATH or at common Homebrew paths (used before WebM→WAV transcode).
#[cfg(not(windows))]
fn probe_ffmpeg_unix() -> bool {
    for p in [
        "/opt/homebrew/bin/ffmpeg",
        "/opt/homebrew/opt/ffmpeg/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/usr/local/opt/ffmpeg/bin/ffmpeg",
    ] {
        if PathBuf::from(p).is_file() {
            return true;
        }
    }
    let Ok(out) = std::process::Command::new("sh")
        .arg("-lc")
        .arg("command -v ffmpeg 2>/dev/null || true")
        .output()
    else {
        return false;
    };
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

/// Install `ffmpeg` via Homebrew (decodes WebM browser audio to WAV for whisper-cli).
#[cfg(all(not(windows), target_os = "macos"))]
fn brew_install_ffmpeg_macos() -> Result<(), String> {
    if std::env::var("ARMARAOS_SKIP_BREW_FFMPEG")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return Err("ARMARAOS_SKIP_BREW_FFMPEG is set".into());
    }
    if std::env::var("ARMARAOS_SKIP_BREW_WHISPER")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return Err("ARMARAOS_SKIP_BREW_WHISPER is set (also skips ffmpeg auto-install)".into());
    }
    if matches!(
        std::env::var("CI").ok().as_deref(),
        Some("true") | Some("1") | Some("True")
    ) {
        return Err("CI is set".into());
    }
    let Some(brew) = homebrew_executable() else {
        return Err("Homebrew not found".into());
    };
    info!("local_voice: installing `ffmpeg` via Homebrew (transcode WebM for local STT)…");
    let status = std::process::Command::new(&brew)
        .args(["install", "ffmpeg"])
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("NONINTERACTIVE", "1")
        .status()
        .map_err(|e| format!("failed to spawn brew: {e}"))?;
    if !status.success() {
        return Err(format!("brew install ffmpeg exited with {status}"));
    }
    Ok(())
}

/// Resolve `$(brew --prefix whisper-cpp)/bin/whisper-cli` when Homebrew is installed.
#[cfg(not(windows))]
fn probe_whisper_via_homebrew_prefix() -> Option<PathBuf> {
    let brew = homebrew_executable().unwrap_or_else(|| PathBuf::from("brew"));
    let out = std::process::Command::new(&brew)
        .args(["--prefix", "whisper-cpp"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let base = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if base.is_empty() {
        return None;
    }
    let p = PathBuf::from(base).join("bin").join("whisper-cli");
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

fn ensure_piper_bundle(client: &Client, voice_root: &Path, piper_root: &Path) -> Result<(), String> {
    let marker = piper_root.join(".extracted_ok");
    if marker.is_file() {
        return Ok(());
    }

    let name = piper_archive_name()?;
    let url = format!("https://github.com/rhasspy/piper/releases/download/{PIPER_TAG}/{name}");
    info!(%url, "local_voice: downloading Piper runtime…");

    let bytes = client
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("GET Piper: {e}"))?
        .bytes()
        .map_err(|e| format!("body: {e}"))?;

    let _ = std::fs::remove_dir_all(piper_root);
    std::fs::create_dir_all(voice_root).map_err(|e| format!("mkdir voice_root: {e}"))?;

    if name.ends_with(".zip") {
        let cursor = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("piper zip: {e}"))?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).map_err(|e| format!("zip: {e}"))?;
            let out_path = piper_root.join(file.name());
            if file.name().ends_with('/') {
                std::fs::create_dir_all(&out_path).ok();
            } else {
                if let Some(p) = out_path.parent() {
                    std::fs::create_dir_all(p).map_err(|e| format!("mkdir: {e}"))?;
                }
                let mut out =
                    std::fs::File::create(&out_path).map_err(|e| format!("create: {e}"))?;
                std::io::copy(&mut file, &mut out).map_err(|e| format!("write: {e}"))?;
            }
        }
    } else if name.ends_with(".tar.gz") {
        let dec = GzDecoder::new(bytes.as_ref());
        let mut archive = tar::Archive::new(dec);
        archive
            .unpack(piper_root)
            .map_err(|e| format!("tar unpack: {e}"))?;
    } else {
        return Err("unsupported Piper archive format".into());
    }

    let _ = std::fs::write(&marker, b"ok\n");
    Ok(())
}

fn piper_archive_name() -> Result<&'static str, String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("windows", "x86_64") => Ok("piper_windows_amd64.zip"),
        ("macos", "aarch64") => Ok("piper_macos_aarch64.tar.gz"),
        ("macos", "x86_64") => Ok("piper_macos_x64.tar.gz"),
        ("linux", "x86_64") => Ok("piper_linux_x86_64.tar.gz"),
        ("linux", "aarch64") => Ok("piper_linux_aarch64.tar.gz"),
        ("linux", "arm") => Ok("piper_linux_armv7l.tar.gz"),
        _ => Err(format!("Piper auto-download: unsupported OS/arch: {os}-{arch}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piper_archive_maps_known_triples() {
        assert!(piper_archive_name().is_ok());
    }
}
