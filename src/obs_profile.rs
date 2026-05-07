//! Writes an OBS Studio profile and scene collection for rscapt.
//!
//! Targets OBS 28+ (WebSocket built-in, obs-websocket v5).
//! We write to a dedicated profile named "rscapt" so we never touch the
//! user's existing profiles.

use crate::config::Config;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const PROFILE_NAME: &str = "rscapt";
const SCENE_NAME: &str = "rscapt";

// ── Public entry point ────────────────────────────────────────────────────────

/// Write the full OBS configuration for rscapt:
/// profile INI, scene collection JSON, and websocket plugin config.
pub fn write(config: &Config) -> Result<()> {
    let obs_appdata = obs_appdata_dir();

    write_profile(config, &obs_appdata)?;
    write_scene_collection(config, &obs_appdata)?;
    write_websocket_config(config, &obs_appdata)?;

    Ok(())
}

/// Returns the path to obs64.exe if it can be found on this machine.
/// Checks the registry (via PowerShell) and a set of common install paths.
pub fn detect_obs() -> Option<PathBuf> {
    // Try registry first — PowerShell gives us the InstallPath key cleanly
    if let Some(path) = detect_via_registry() {
        return Some(path);
    }
    // Fall back to common install paths
    let candidates = [
        r"C:\Program Files\obs-studio\bin\64bit\obs64.exe",
        r"C:\Program Files (x86)\obs-studio\bin\64bit\obs64.exe",
    ];
    for candidate in &candidates {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Returns true if obs64.exe is currently running.
pub fn is_running() -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq obs64.exe", "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("obs64.exe"))
        .unwrap_or(false)
}

/// Launch OBS in the background with the rscapt profile, minimised to tray,
/// replay buffer auto-started.
pub fn launch(exe_path: &Path) -> Result<()> {
    std::process::Command::new(exe_path)
        .args([
            "--profile",          PROFILE_NAME,
            "--collection",       SCENE_NAME,
            "--minimize-to-tray",
            "--startreplaybuffer",
        ])
        .spawn()
        .with_context(|| format!("launching OBS from {}", exe_path.display()))?;
    Ok(())
}

// ── Profile INI ───────────────────────────────────────────────────────────────

fn write_profile(config: &Config, obs_appdata: &Path) -> Result<()> {
    let dir = obs_appdata
        .join("basic")
        .join("profiles")
        .join(PROFILE_NAME);
    std::fs::create_dir_all(&dir)?;

    let ini = build_profile_ini(config);
    std::fs::write(dir.join("basic.ini"), ini)
        .context("writing OBS profile basic.ini")?;

    Ok(())
}

fn build_profile_ini(config: &Config) -> String {
    let output_dir = &config.output_dir;
    let rb_seconds = config.replay_buffer_seconds;
    // OBS replay buffer size cap in MB (generous — actual limit is time-based)
    let rb_size_mb = 4096u32;
    let ws_port = config.obs_port;
    let ws_password = &config.obs_password;
    let ws_auth = if ws_password.is_empty() { "false" } else { "true" };

    format!(
        r#"[General]
Name={PROFILE_NAME}

[Video]
BaseCX=1920
BaseCY=1080
OutputCX=1920
OutputCY=1080
FPSType=0
FPSCommon=60

[Audio]
SampleRate=48000
ChannelSetup=Stereo

[Output]
Mode=Simple

[SimpleOutput]
FilePath={output_dir}
RecFormat=mkv
RecQuality=Stream
RecEncoder=none
RecRB=true
RecRBTime={rb_seconds}
RecRBSize={rb_size_mb}
RecRBPrefix=Replay

[obs-websocket]
ServerEnabled=true
ServerPort={ws_port}
AuthRequired={ws_auth}
ServerPassword={ws_password}
AlertsEnabled=false
DebugLog=false
"#
    )
}

// ── Scene collection ──────────────────────────────────────────────────────────

fn write_scene_collection(config: &Config, obs_appdata: &Path) -> Result<()> {
    let dir = obs_appdata.join("basic").join("scenes");
    std::fs::create_dir_all(&dir)?;

    let json = build_scene_json(config);
    std::fs::write(dir.join(format!("{SCENE_NAME}.json")), json)
        .context("writing OBS scene collection")?;

    Ok(())
}

fn build_scene_json(config: &Config) -> String {
    let (source_id, source_name, source_settings) = match config.capture_source.as_str() {
        "display" => (
            "monitor_capture",
            "Display Capture",
            r#"{"monitor": 0, "capture_cursor": true}"#,
        ),
        _ => (
            "game_capture",
            "Game Capture",
            r#"{"capture_mode": "any_fullscreen", "allow_transparency": false, "capture_cursor": true}"#,
        ),
    };

    // Minimal scene collection — single scene with one capture source.
    // Desktop audio (wasapi_output_capture) is always included.
    format!(
        r#"{{
  "current_program_scene": "{SCENE_NAME}",
  "current_scene": "{SCENE_NAME}",
  "current_transition": "FadeTransition",
  "transition_duration": 300,
  "name": "{SCENE_NAME}",
  "sources": [
    {{
      "id": "scene",
      "versioned_id": "scene",
      "name": "{SCENE_NAME}",
      "settings": {{
        "id_counter": 2,
        "items": [
          {{
            "name": "{source_name}",
            "visible": true,
            "locked": false,
            "pos": {{"x": 0.0, "y": 0.0}},
            "bounds": {{"x": 0.0, "y": 0.0}},
            "bounds_type": 0,
            "bounds_align": 0,
            "scale": {{"x": 1.0, "y": 1.0}},
            "align": 5,
            "rot": 0.0,
            "id": 1,
            "group_item_id": 0
          }},
          {{
            "name": "Desktop Audio",
            "visible": true,
            "locked": false,
            "pos": {{"x": 0.0, "y": 0.0}},
            "bounds": {{"x": 0.0, "y": 0.0}},
            "bounds_type": 0,
            "bounds_align": 0,
            "scale": {{"x": 1.0, "y": 1.0}},
            "align": 5,
            "rot": 0.0,
            "id": 2,
            "group_item_id": 0
          }}
        ]
      }},
      "volume": 1.0,
      "muted": false,
      "enabled": true
    }},
    {{
      "id": "{source_id}",
      "versioned_id": "{source_id}",
      "name": "{source_name}",
      "settings": {source_settings},
      "volume": 1.0,
      "muted": false,
      "enabled": true
    }},
    {{
      "id": "wasapi_output_capture",
      "versioned_id": "wasapi_output_capture",
      "name": "Desktop Audio",
      "settings": {{"device_id": "default", "use_device_timing": false}},
      "volume": 1.0,
      "muted": false,
      "enabled": true
    }}
  ]
}}"#
    )
}

// ── WebSocket plugin config ───────────────────────────────────────────────────

fn write_websocket_config(config: &Config, obs_appdata: &Path) -> Result<()> {
    let dir = obs_appdata.join("plugin_config").join("obs-websocket");
    std::fs::create_dir_all(&dir)?;

    let auth = !config.obs_password.is_empty();
    let json = format!(
        r#"{{
  "alerts_enabled": false,
  "auth_required": {auth},
  "debug_log": false,
  "server_enabled": true,
  "server_password": "{password}",
  "server_port": {port}
}}"#,
        auth = auth,
        password = config.obs_password.replace('"', "\\\""),
        port = config.obs_port,
    );

    std::fs::write(dir.join("config.json"), json)
        .context("writing obs-websocket config")?;

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn obs_appdata_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("obs-studio")
}

fn detect_via_registry() -> Option<PathBuf> {
    // Query registry via PowerShell — avoids pulling in a Windows-specific crate
    let output = std::process::Command::new("powershell")
        .args([
            "-NonInteractive",
            "-NoProfile",
            "-Command",
            r"(Get-ItemProperty 'HKLM:\SOFTWARE\OBS Studio' -ErrorAction SilentlyContinue).InstallPath",
        ])
        .output()
        .ok()?;

    let raw = String::from_utf8_lossy(&output.stdout);
    let install_path = raw.trim();
    if install_path.is_empty() {
        return None;
    }
    let exe = PathBuf::from(install_path)
        .join("bin")
        .join("64bit")
        .join("obs64.exe");
    if exe.exists() { Some(exe) } else { None }
}
