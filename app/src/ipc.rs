//! IPC bridge: connects to the rscapt daemon and relays messages to/from the frontend.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use uuid::Uuid;

// ── Message types (mirrored from rscapt ipc.rs) ───────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type")]
pub enum ServerMessage {
    Snapshot { jobs: Vec<serde_json::Value> },
    JobUpdate { job: serde_json::Value },
    Cancelled { id: Uuid, success: bool },
    ClipLibrary { clips: Vec<serde_json::Value> },
    ClipUpdated { clips: Vec<serde_json::Value> },
    UpdateAvailable { version: String },
    Error { message: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    GetJobs,
    GetClips,
    Cancel { id: Uuid },
    PostProcess { clip_path: PathBuf, effects: Vec<serde_json::Value> },
    Compress { clip_path: PathBuf, options: serde_json::Value },
    Share { clip_path: PathBuf, expiry: String },
}

// ── Daemon auto-start ─────────────────────────────────────────────────────────

pub fn maybe_start_daemon(port: u16) {
    if std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let daemon = exe.parent().unwrap_or(exe.as_path()).join("rscapt.exe");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new(&daemon)
            .arg("daemon")
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new(&daemon).arg("daemon").spawn();
    }
}

// ── IPC connection ────────────────────────────────────────────────────────────

pub struct DaemonConn {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
}

impl DaemonConn {
    pub async fn connect(port: u16) -> Result<Self> {
        let stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .context("connecting to daemon")?;
        let (r, w) = stream.into_split();
        Ok(Self { reader: BufReader::new(r), writer: w })
    }

    pub async fn send(&mut self, msg: &ClientMessage) -> Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        Ok(())
    }

    pub async fn next(&mut self) -> Result<Option<serde_json::Value>> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 { return Ok(None); }
        Ok(Some(serde_json::from_str(line.trim_end())?))
    }
}

// ── Bridge loop ───────────────────────────────────────────────────────────────

/// Wait for the daemon then run the bridge until the connection drops.
pub async fn run_bridge(app: AppHandle, port: u16) {
    loop {
        match try_bridge(&app, port).await {
            Ok(()) => {
                let _ = app.emit("daemon-disconnected", ());
            }
            Err(e) => {
                tracing::warn!("IPC bridge error: {e}");
                let _ = app.emit("daemon-disconnected", ());
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn try_bridge(app: &AppHandle, port: u16) -> Result<()> {
    // Wait up to 10 s for daemon
    let mut conn = wait_for_daemon(port).await?;

    conn.send(&ClientMessage::GetJobs).await?;
    conn.send(&ClientMessage::GetClips).await?;
    let _ = app.emit("daemon-connected", ());

    loop {
        match conn.next().await? {
            None => break,
            Some(msg) => {
                let _ = app.emit("daemon-message", &msg);
            }
        }
    }
    Ok(())
}

async fn wait_for_daemon(port: u16) -> Result<DaemonConn> {
    for _ in 0..20 {
        if let Ok(conn) = DaemonConn::connect(port).await {
            return Ok(conn);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    anyhow::bail!("daemon did not start within 10 s")
}
