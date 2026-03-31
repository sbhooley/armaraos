//! Build automation tasks for the OpenFang workspace.
//!
//! This is the home for "glue" steps that don't belong in crates themselves,
//! especially packaging-time tasks.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "xtask")]
#[command(about = "Workspace automation tasks", long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Download a pinned `ainativelang` wheel into the desktop bundle resources.
    ///
    /// The output location is:
    ///   crates/openfang-desktop/resources/ainl/
    ///
    /// This step is intended to run in the release pipeline before the Tauri bundle.
    BundleAinlWheel {
        /// AINL version to download from PyPI.
        #[arg(long, default_value = "1.4.0")]
        version: String,

        /// If set, remove any existing `ainativelang-*-py3-none-any.whl` before downloading.
        #[arg(long, default_value_t = true)]
        clean: bool,

        /// Extra pip index URL (for private mirrors / caching proxies).
        #[arg(long)]
        extra_index_url: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::BundleAinlWheel {
            version,
            clean,
            extra_index_url,
        } => bundle_ainl_wheel(&version, clean, extra_index_url.as_deref()),
    }
}

fn repo_root() -> Result<PathBuf> {
    // Works in CI and locally as long as invoked somewhere inside the repo.
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("Running git rev-parse to detect repo root")?;
    if !out.status.success() {
        return Err(anyhow!(
            "git rev-parse failed (exit={})",
            out.status.code().unwrap_or(-1)
        ));
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return Err(anyhow!("git rev-parse returned empty repo root"));
    }
    Ok(PathBuf::from(s))
}

fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("Running {cmd:?}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "Command failed (exit={})\nstdout:\n{}\nstderr:\n{}",
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

fn remove_existing_wheels(dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for ent in std::fs::read_dir(dir).with_context(|| format!("Reading {dir:?}"))? {
        let ent = ent?;
        let p = ent.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with("ainativelang-") && name.ends_with("-py3-none-any.whl") {
            std::fs::remove_file(&p).with_context(|| format!("Removing existing wheel {p:?}"))?;
        }
    }
    Ok(())
}

fn bundle_ainl_wheel(version: &str, clean: bool, extra_index_url: Option<&str>) -> Result<()> {
    let root = repo_root()?;
    let out_dir = root
        .join("crates")
        .join("openfang-desktop")
        .join("resources")
        .join("ainl");
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("Creating resources dir {out_dir:?}"))?;

    if clean {
        remove_existing_wheels(&out_dir)?;
    }

    // Use pip to download a pure-Python wheel that we can install offline later.
    // We keep this small: just the `ainativelang` wheel itself (no dependencies).
    let mut args = vec![
        "-m",
        "pip",
        "download",
        "--only-binary",
        ":all:",
        "--no-deps",
        "--dest",
    ];
    let out_dir_str = out_dir
        .to_str()
        .ok_or_else(|| anyhow!("Non-utf8 output dir path: {out_dir:?}"))?;
    args.push(out_dir_str);

    // Pin exact version.
    let spec = format!("ainativelang=={version}");
    args.push(&spec);

    if let Some(url) = extra_index_url {
        args.push("--extra-index-url");
        args.push(url);
    }

    // Try python3 first, then python (Windows runners).
    let mut cmd = Command::new("python3");
    cmd.args(args.clone());
    if run(&mut cmd).is_err() {
        let mut cmd = Command::new("python");
        cmd.args(args);
        run(&mut cmd)?;
    }

    // Sanity: verify at least one wheel exists after download.
    let mut found = false;
    for ent in std::fs::read_dir(&out_dir).with_context(|| format!("Reading {out_dir:?}"))? {
        let ent = ent?;
        let p = ent.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with("ainativelang-") && name.ends_with("-py3-none-any.whl") {
            found = true;
            break;
        }
    }
    if !found {
        return Err(anyhow!(
            "No ainativelang wheel found in {out_dir:?} after download"
        ));
    }

    println!("Bundled ainativelang wheel into {}", out_dir.display());
    Ok(())
}
