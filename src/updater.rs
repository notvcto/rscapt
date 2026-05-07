//! Self-update: check GitHub releases and swap in a newer exe.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

const REPO: &str = "notvcto/rscapt";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: String,
    pub download_url: String,
}

/// Check GitHub for a newer release. Returns `Some(info)` if one exists,
/// `None` if already up to date.
pub async fn check_latest() -> Result<Option<ReleaseInfo>> {
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        assets: Vec<Asset>,
    }

    let client = reqwest::Client::builder()
        .user_agent(format!("rscapt/{CURRENT}"))
        .timeout(Duration::from_secs(10))
        .build()?;

    let release: Release = client
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .send()
        .await
        .context("fetching latest release info")?
        .json()
        .await
        .context("parsing release JSON")?;

    let latest = release.tag_name.trim_start_matches('v');
    if !is_newer(latest, CURRENT) {
        return Ok(None);
    }

    let asset = release
        .assets
        .iter()
        .find(|a| a.name.contains("windows-x64") && a.name.ends_with(".exe"))
        .ok_or_else(|| {
            anyhow::anyhow!("No Windows exe asset in release {}", release.tag_name)
        })?;

    Ok(Some(ReleaseInfo {
        version: release.tag_name,
        download_url: asset.browser_download_url.clone(),
    }))
}

/// Download the new exe to %TEMP% then launch a detached PowerShell snippet
/// that overwrites the installed exe after this process exits.
pub async fn download_and_swap(release: &ReleaseInfo) -> Result<()> {
    let install_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("rscapt");
    let dest_exe = install_dir.join("rscapt.exe");
    let tmp = std::env::temp_dir().join("rscapt-update.exe");

    println!("Downloading {}...", release.version);

    let client = reqwest::Client::builder()
        .user_agent(format!("rscapt/{CURRENT}"))
        .build()?;

    let mut resp = client
        .get(&release.download_url)
        .send()
        .await
        .context("starting download")?;

    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&tmp).await.context("creating temp file")?;
    let mut done = 0u64;
    let mut last_pct = 0u64;

    while let Some(chunk) = resp.chunk().await? {
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
        done += chunk.len() as u64;
        if total > 0 {
            let pct = done * 100 / total;
            if pct >= last_pct + 10 {
                eprint!("\r  {pct}%");
                last_pct = pct;
            }
        }
    }
    eprintln!();
    drop(file);

    // Detached PowerShell: wait for this process to exit, then swap the exe.
    let src = tmp.to_string_lossy().replace('\'', "''");
    let dst = dest_exe.to_string_lossy().replace('\'', "''");
    let script = format!(
        "Stop-Process -Name rscapt -Force -ErrorAction SilentlyContinue; \
         Start-Sleep -Seconds 2; \
         Copy-Item -Force '{src}' '{dst}'; \
         Remove-Item '{src}' -ErrorAction SilentlyContinue"
    );

    std::process::Command::new("powershell")
        .args([
            "-WindowStyle",    "Hidden",
            "-NonInteractive",
            "-NoProfile",
            "-Command",        &script,
        ])
        .spawn()
        .context("launching update swap script")?;

    println!("Update staged — rscapt will be replaced on next launch.");
    println!("Run `rscapt daemon` (or restart) to use {}.", release.version);

    Ok(())
}

/// `rscapt update` entry point: check, print status, download if newer found.
pub async fn run_update() -> Result<()> {
    println!("Checking for updates (current: v{CURRENT})...");
    match check_latest().await? {
        None => println!("Already up to date."),
        Some(release) => {
            println!("New version available: {}", release.version);
            download_and_swap(&release).await?;
        }
    }
    Ok(())
}

// ── Version comparison ────────────────────────────────────────────────────────

fn is_newer(latest: &str, current: &str) -> bool {
    parse_semver(latest) > parse_semver(current)
}

fn parse_semver(v: &str) -> (u32, u32, u32) {
    let mut it = v.splitn(3, '.').map(|p| p.parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}
