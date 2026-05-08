mod ipc;

use ipc::{ClientMessage, maybe_start_daemon};
use serde::Deserialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

// ── Shared state ──────────────────────────────────────────────────────────────

struct DaemonWriter(Mutex<Option<ipc::DaemonConn>>);

// ── Tauri commands (frontend → daemon) ───────────────────────────────────────

#[tauri::command]
async fn send_cmd(
    app: AppHandle,
    msg: serde_json::Value,
) -> Result<(), String> {
    // Spin up a short-lived write connection for commands
    let port = get_port(&app);
    let mut conn = ipc::DaemonConn::connect(port)
        .await
        .map_err(|e| e.to_string())?;
    let cmd: ClientMessage = serde_json::from_value(msg).map_err(|e| e.to_string())?;
    conn.send(&cmd).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn minimize_window(window: tauri::Window) {
    let _ = window.minimize();
}

#[tauri::command]
async fn maximize_window(window: tauri::Window) {
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    } else {
        let _ = window.maximize();
    }
}

#[tauri::command]
async fn close_window(window: tauri::Window) {
    let _ = window.close();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn get_port(app: &AppHandle) -> u16 {
    // Read from config; fall back to default
    let config_path = dirs::data_dir()
        .unwrap_or_default()
        .join("rscapt")
        .join("config.json");
    if let Ok(data) = std::fs::read_to_string(&config_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
            if let Some(p) = v["ipc_port"].as_u64() {
                return p as u16;
            }
        }
    }
    19874 // default
}

// ── App entry point ───────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            let port = get_port(&handle);
            maybe_start_daemon(port);
            tauri::async_runtime::spawn(async move {
                ipc::run_bridge(handle, port).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            send_cmd,
            minimize_window,
            maximize_window,
            close_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running rscapt app");
}
