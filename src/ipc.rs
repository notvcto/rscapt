//! Lightweight IPC between the daemon and any attached TUI instance.
//! Protocol: newline-delimited JSON over a localhost TCP socket.

use crate::{
    clips::{Clip, ClipStore},
    job::{CompressOptions, Effect, Job, JobKind},
    queue::{Queue, QueueEvent},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{
        TcpListener, TcpStream,
        tcp::OwnedWriteHalf,
    },
};
use tracing::{info, warn};
use uuid::Uuid;

/// Messages the TUI can send to the daemon.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    GetJobs,
    GetClips,
    Cancel { id: Uuid },
    PostProcess { clip_path: PathBuf, effects: Vec<Effect> },
    Compress { clip_path: PathBuf, options: CompressOptions },
    Share { clip_path: PathBuf, expiry: String },
}

/// Messages the daemon sends to the TUI.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    Snapshot { jobs: Vec<Job> },
    JobUpdate { job: Job },
    Cancelled { id: Uuid, success: bool },
    ClipLibrary { clips: Vec<Clip> },
    ClipUpdated { clips: Vec<Clip> },
    UpdateAvailable { version: String },
    Error { message: String },
}

// ── Server (runs inside daemon) ───────────────────────────────────────────────

pub async fn serve(
    port: u16,
    queue: Arc<Queue>,
    clips: Arc<ClipStore>,
    update_rx: tokio::sync::watch::Receiver<Option<String>>,
) -> Result<()> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!("IPC server listening on {addr}");

    loop {
        let (stream, peer) = listener.accept().await?;
        info!("TUI connected from {peer}");
        let queue = queue.clone();
        let clips = clips.clone();
        let urx = update_rx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, queue, clips, urx).await {
                warn!("IPC client error: {e}");
            }
        });
    }
}

async fn handle_client(
    stream: TcpStream,
    queue: Arc<Queue>,
    clips: Arc<ClipStore>,
    mut update_rx: tokio::sync::watch::Receiver<Option<String>>,
) -> Result<()> {
    let mut job_events = queue.subscribe();
    let mut clip_events = clips.subscribe();
    let (read_half, write_half) = stream.into_split();
    let writer = Arc::new(tokio::sync::Mutex::new(write_half));

    // Send initial job snapshot
    {
        let jobs = queue.snapshot().await;
        write_msg(&writer, &ServerMessage::Snapshot { jobs }).await?;
    }

    // If an update is already known, notify immediately.
    // Extract before .await so the RwLockReadGuard isn't held across the yield point.
    let current_update = update_rx.borrow().clone();
    if let Some(version) = current_update {
        write_msg(&writer, &ServerMessage::UpdateAvailable { version }).await?;
    }

    // Task: forward queue events → client
    let writer_jobs = writer.clone();
    let job_task = tokio::spawn(async move {
        loop {
            match job_events.recv().await {
                Ok(event) => {
                    let msg = match event {
                        QueueEvent::JobAdded(job) | QueueEvent::JobUpdated(job) => {
                            ServerMessage::JobUpdate { job }
                        }
                    };
                    if write_msg(&writer_jobs, &msg).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Skipped some progress updates — not fatal, continue
                }
                Err(_) => break,
            }
        }
    });

    // Task: forward clip store events → client
    let writer_clips = writer.clone();
    let clip_task = tokio::spawn(async move {
        while let Ok(snapshot) = clip_events.recv().await {
            let msg = ServerMessage::ClipUpdated { clips: snapshot };
            if write_msg(&writer_clips, &msg).await.is_err() {
                break;
            }
        }
    });

    // Task: forward update notifications → client
    let writer_update = writer.clone();
    let update_task = tokio::spawn(async move {
        loop {
            if update_rx.changed().await.is_err() { break; }
            // Clone before .await so the RwLockReadGuard is dropped first
            let version = update_rx.borrow().clone();
            if let Some(version) = version {
                if write_msg(&writer_update, &ServerMessage::UpdateAvailable { version })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    });

    // Read client commands
    let mut lines = BufReader::new(read_half).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        match serde_json::from_str::<ClientMessage>(&line) {
            Ok(msg) => {
                if let Err(e) = dispatch(msg, &queue, &clips, &writer).await {
                    warn!("IPC dispatch error: {e}");
                    write_msg(&writer, &ServerMessage::Error { message: e.to_string() })
                        .await
                        .ok();
                }
            }
            Err(e) => warn!("IPC parse error: {e}"),
        }
    }

    job_task.abort();
    clip_task.abort();
    update_task.abort();
    Ok(())
}

async fn dispatch(
    msg: ClientMessage,
    queue: &Arc<Queue>,
    clips: &Arc<ClipStore>,
    writer: &Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
) -> Result<()> {
    match msg {
        ClientMessage::GetJobs => {
            let jobs = queue.snapshot().await;
            write_msg(writer, &ServerMessage::Snapshot { jobs }).await?;
        }
        ClientMessage::GetClips => {
            let clip_list = clips.snapshot().await;
            write_msg(writer, &ServerMessage::ClipLibrary { clips: clip_list }).await?;
        }
        ClientMessage::Cancel { id } => {
            let success = queue.cancel(id).await;
            write_msg(writer, &ServerMessage::Cancelled { id, success }).await?;
        }
        ClientMessage::PostProcess { clip_path, effects } => {
            let output = derive_output_path(&clip_path, "pp");
            let job = Job::new(JobKind::PostProcess { effects }, clip_path, output);
            queue.push(job).await;
        }
        ClientMessage::Compress { clip_path, options } => {
            let suffix = format!("c_{}", options.codec.ffmpeg_codec().replace('_', ""));
            let output = derive_output_path(&clip_path, &suffix);
            let job = Job::new(JobKind::Compress(options), clip_path, output);
            queue.push(job).await;
        }
        ClientMessage::Share { clip_path, expiry } => {
            let output = clip_path.clone();
            let job = Job::new(JobKind::Share { expiry }, clip_path, output);
            queue.push(job).await;
        }
    }
    Ok(())
}

async fn write_msg(
    writer: &Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    msg: &ServerMessage,
) -> Result<()> {
    let mut line = serde_json::to_string(msg)?;
    line.push('\n');
    writer.lock().await.write_all(line.as_bytes()).await?;
    Ok(())
}

// ── Client (runs inside TUI) ──────────────────────────────────────────────────

pub struct IpcClient {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl IpcClient {
    pub async fn connect(port: u16) -> Result<Self> {
        let stream = TcpStream::connect(format!("127.0.0.1:{port}")).await?;
        let (read, write) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(read),
            writer: write,
        })
    }

    pub async fn send(&mut self, msg: &ClientMessage) -> Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        Ok(())
    }

    /// Returns the next message from the daemon, or `None` if the connection closed.
    pub async fn next_message(&mut self) -> Result<Option<ServerMessage>> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(line.trim_end())?))
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Derive an output path by appending a suffix to the file stem.
/// e.g. `clip_1440p.mp4` + `"pp"` → `clip_1440p_pp.mp4`
fn derive_output_path(source: &PathBuf, suffix: &str) -> PathBuf {
    let parent = source.parent().unwrap_or(std::path::Path::new("."));
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let ext = source.extension().unwrap_or_default().to_string_lossy();
    parent.join(format!("{stem}_{suffix}.{ext}"))
}
