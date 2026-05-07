use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_output_dir() -> String {
    dirs::video_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("Captures")
        .to_string_lossy()
        .into_owned()
}

fn default_temp_dir() -> String {
    std::env::temp_dir()
        .join("rscapt")
        .to_string_lossy()
        .into_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // OBS WebSocket
    #[serde(default = "default_obs_host")]
    pub obs_host: String,
    #[serde(default = "default_obs_port")]
    pub obs_port: u16,
    #[serde(default)]
    pub obs_password: String,

    // Paths
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    #[serde(default = "default_temp_dir")]
    pub temp_dir: String,

    // Upscale — raw pass, no colour grading here
    #[serde(default = "default_resolution")]
    pub upscale_resolution: String,
    #[serde(default = "default_filter")]
    pub upscale_filter: String,
    #[serde(default = "default_encoder")]
    pub encoder: String,
    #[serde(default = "default_nvenc_quality")]
    pub nvenc_quality: u8,

    // IPC
    #[serde(default = "default_ipc_port")]
    pub ipc_port: u16,

    // OBS management
    /// Path to obs64.exe. Empty = not configured.
    #[serde(default)]
    pub obs_exe_path: String,
    /// If true, the daemon owns the OBS process: launches it on start, kills it on exit.
    #[serde(default)]
    pub obs_managed: bool,
    /// Replay buffer duration written to the rscapt OBS profile.
    #[serde(default = "default_replay_buffer_seconds")]
    pub replay_buffer_seconds: u32,
    /// Capture source type written to the rscapt OBS profile: "game" or "display".
    #[serde(default = "default_capture_source")]
    pub capture_source: String,
}

fn default_obs_host() -> String { "127.0.0.1".into() }
fn default_obs_port() -> u16 { 4455 }
fn default_resolution() -> String { "2560x1440".into() }
fn default_filter() -> String { "lanczos".into() }
fn default_encoder() -> String { "h264_nvenc".into() }
fn default_nvenc_quality() -> u8 { 18 }
fn default_ipc_port() -> u16 { 7373 }
fn default_replay_buffer_seconds() -> u32 { 30 }
fn default_capture_source() -> String { "game".into() }

impl Default for Config {
    fn default() -> Self {
        Self {
            obs_host: default_obs_host(),
            obs_port: default_obs_port(),
            obs_password: String::new(),
            output_dir: default_output_dir(),
            temp_dir: default_temp_dir(),
            upscale_resolution: default_resolution(),
            upscale_filter: default_filter(),
            encoder: default_encoder(),
            nvenc_quality: default_nvenc_quality(),
            ipc_port: default_ipc_port(),
            obs_exe_path: String::new(),
            obs_managed: false,
            replay_buffer_seconds: default_replay_buffer_seconds(),
            capture_source: default_capture_source(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rscapt")
            .join("config.json")
    }

    pub fn data_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rscapt")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        if path.exists() {
            let text = std::fs::read_to_string(&path)?;
            let cfg: Self = serde_json::from_str(&text)?;
            eprintln!("[rscapt] loaded config from {}", path.display());
            Ok(cfg)
        } else {
            eprintln!("[rscapt] no config file at {}, using defaults", path.display());
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}
