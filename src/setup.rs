//! Setup logic — called by the wizard once the user has answered all questions.
//! No UI here; see wizard.rs for the ratatui front-end.

use crate::{config::Config, installer, obs_profile};
use anyhow::{Context, Result};
use std::path::PathBuf;

// ── Options collected by the wizard ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObsChoice {
    /// Download OBS portable automatically from GitHub releases.
    Download,
    /// Use an existing OBS installation at a known path.
    Existing(String),
    /// Skip — user will configure obs_exe_path manually later.
    Skip,
}

#[derive(Debug, Clone)]
pub struct SetupOptions {
    pub obs: ObsChoice,
    pub output_dir: String,
    pub buffer_secs: u32,
    /// "game" or "display"
    pub capture_source: String,
    pub autostart: bool,
}

// ── Apply ─────────────────────────────────────────────────────────────────────

/// Apply the collected options: resolve/download OBS, write OBS profile,
/// save config, and optionally register the autostart.
///
/// Prints progress to stdout — call this after the wizard TUI has exited.
pub fn apply(opts: &SetupOptions) -> Result<()> {
    let mut config = Config::load().unwrap_or_default();

    config.output_dir     = opts.output_dir.clone();
    config.replay_buffer_seconds = opts.buffer_secs;
    config.capture_source = opts.capture_source.clone();

    // ── OBS ───────────────────────────────────────────────────────────────────
    match &opts.obs {
        ObsChoice::Download => {
            println!("Downloading OBS Studio...");
            let exe = download_obs()?;
            config.obs_exe_path = exe.to_string_lossy().into_owned();
            config.obs_managed  = true;
        }
        ObsChoice::Existing(path) => {
            config.obs_exe_path = path.clone();
            config.obs_managed  = true;
        }
        ObsChoice::Skip => {
            config.obs_managed = false;
        }
    }

    // ── OBS profile ───────────────────────────────────────────────────────────
    if config.obs_managed {
        print!("Writing OBS profile... ");
        obs_profile::write(&config).context("writing OBS profile")?;
        println!("done");
    }

    // ── Save config ───────────────────────────────────────────────────────────
    print!("Saving config... ");
    config.save().context("saving rscapt config")?;
    println!("done");

    // ── Autostart + Start Menu ────────────────────────────────────────────────
    if opts.autostart {
        print!("Registering autostart... ");
        installer::install().context("registering autostart")?;
        println!("done");
    }

    Ok(())
}

// ── OBS download ──────────────────────────────────────────────────────────────

fn download_obs() -> Result<PathBuf> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let client = reqwest::Client::builder()
            .user_agent("rscapt-setup")
            .build()?;

        println!("Fetching latest OBS release info...");
        let release: serde_json::Value = client
            .get("https://api.github.com/repos/obsproject/obs-studio/releases/latest")
            .send()
            .await?
            .json()
            .await?;

        let tag = release["tag_name"].as_str().unwrap_or("?").to_owned();

        let asset = release["assets"]
            .as_array()
            .and_then(|a| {
                a.iter().find(|a| {
                    let n = a["name"].as_str().unwrap_or("");
                    n.contains("Windows") && n.ends_with(".zip") && !n.contains("PDB")
                })
            })
            .ok_or_else(|| anyhow::anyhow!("No Windows zip found in OBS release"))?;

        let url   = asset["browser_download_url"].as_str().unwrap_or("").to_owned();
        let name  = asset["name"].as_str().unwrap_or("OBS.zip").to_owned();
        let size  = asset["size"].as_u64().unwrap_or(0);

        println!("Downloading {} v{tag} ({})", name, fmt_bytes(size));

        let install_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rscapt")
            .join("obs");
        std::fs::create_dir_all(&install_dir)?;

        let mut resp = client.get(&url).send().await?;
        let zip_path = install_dir.join(&name);
        let mut file = tokio::fs::File::create(&zip_path).await?;
        let mut done = 0u64;
        let mut last = 0u64;
        while let Some(chunk) = resp.chunk().await? {
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
            done += chunk.len() as u64;
            if size > 0 {
                let pct = done * 100 / size;
                if pct >= last + 10 {
                    eprint!("\r  {pct}%");
                    last = pct;
                }
            }
        }
        eprintln!();

        println!("Extracting...");
        let zp = zip_path.clone();
        let id = install_dir.clone();
        tokio::task::spawn_blocking(move || extract_zip(&zp, &id)).await??;
        std::fs::remove_file(&zip_path).ok();

        let exe = find_file(&install_dir, "obs64.exe")
            .ok_or_else(|| anyhow::anyhow!("obs64.exe not found after extraction"))?;
        println!("OBS installed to {}", exe.display());
        Ok(exe)
    })
}

fn extract_zip(zip_path: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let out = dest.join(entry.mangled_name());
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
            std::io::copy(&mut entry, &mut std::fs::File::create(&out)?)?;
        }
    }
    Ok(())
}

fn find_file(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(f) = find_file(&p, name) { return Some(f); }
        } else if p.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(p);
        }
    }
    None
}

fn fmt_bytes(b: u64) -> String {
    if b >= 1_048_576 { format!("{:.0} MB", b as f64 / 1_048_576.0) }
    else { format!("{:.0} KB", b as f64 / 1024.0) }
}
