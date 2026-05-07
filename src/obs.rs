use crate::config::Config;
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

/// Fired when OBS saves the replay buffer.
#[derive(Debug, Clone)]
pub struct ReplayBufferSaved {
    pub path: std::path::PathBuf,
}

// OBS WebSocket protocol is parsed directly from serde_json::Value
// to avoid fighting with the tagged enum layout of the protocol.

fn make_auth_string(password: &str, salt: &str, challenge: &str) -> String {
    // OBS auth: base64(sha256(base64(sha256(password + salt)) + challenge))
    let mut h1 = Sha256::new();
    h1.update(password.as_bytes());
    h1.update(salt.as_bytes());
    let secret = BASE64.encode(h1.finalize());

    let mut h2 = Sha256::new();
    h2.update(secret.as_bytes());
    h2.update(challenge.as_bytes());
    BASE64.encode(h2.finalize())
}

pub async fn run(config: Arc<Config>, tx: mpsc::Sender<ReplayBufferSaved>) -> Result<()> {
    let url = format!("ws://{}:{}", config.obs_host, config.obs_port);
    info!(url = %url, "OBS WebSocket task started");

    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        info!(attempt, "Connecting to OBS");
        match connect_and_listen(&url, &config.obs_password, tx.clone()).await {
            Ok(()) => {
                info!("OBS connection closed cleanly, reconnecting…");
                attempt = 0;
            }
            Err(e) => {
                // Log as error on first failure, warn on subsequent (avoids log spam)
                if attempt == 1 {
                    error!(error = %e, "OBS connection failed — is OBS running with obs-websocket enabled?");
                } else {
                    warn!(attempt, error = %e, "OBS reconnection failed");
                }
            }
        }
        let backoff = std::cmp::min(5 * attempt, 30);
        info!(seconds = backoff, "Waiting before reconnect");
        tokio::time::sleep(std::time::Duration::from_secs(backoff as u64)).await;
    }
}

async fn connect_and_listen(
    url: &str,
    password: &str,
    tx: mpsc::Sender<ReplayBufferSaved>,
) -> Result<()> {
    let (ws, _) = connect_async(url).await?;
    info!("TCP connection established");
    let (mut sink, mut stream) = ws.split();

    while let Some(msg) = stream.next().await {
        let msg = msg?;
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(frame) => {
                info!(reason = ?frame, "OBS sent Close frame");
                return Ok(());
            }
            _ => continue,
        };

        let parsed: Value = serde_json::from_str(&text)?;
        let op = parsed["op"].as_u64().unwrap_or(99);

        match op {
            // Hello — send Identify
            0 => {
                let needs_auth = parsed["d"]["authentication"].is_object();
                info!(needs_auth, "Received Hello from OBS");

                let auth_string = if needs_auth {
                    if password.is_empty() {
                        warn!("OBS requires authentication but no password is configured — set obs_password in config");
                    }
                    let salt = parsed["d"]["authentication"]["salt"].as_str().unwrap_or("");
                    let challenge = parsed["d"]["authentication"]["challenge"].as_str().unwrap_or("");
                    debug!("Computing auth response");
                    Some(make_auth_string(password, salt, challenge))
                } else {
                    None
                };

                let identify = json!({
                    "op": 1,
                    "d": {
                        "rpcVersion": 1,
                        "authentication": auth_string,
                        // Subscribe to Outputs event category (bit 6) — covers ReplayBufferSaved
                        "eventSubscriptions": 1 << 6
                    }
                });
                sink.send(Message::text(identify.to_string())).await?;
                debug!("Sent Identify");
            }

            // Identified — handshake complete
            2 => {
                let rpc = parsed["d"]["negotiatedRpcVersion"].as_u64().unwrap_or(0);
                info!(rpc_version = rpc, "OBS WebSocket handshake complete — listening for ReplayBufferSaved");
            }

            // Event
            5 => {
                let event_type = parsed["d"]["eventType"].as_str().unwrap_or("");
                debug!(event_type, "OBS event received");

                if event_type == "ReplayBufferSaved" {
                    if let Some(path_str) = parsed["d"]["eventData"]["savedReplayPath"].as_str() {
                        info!(path = %path_str, "Replay buffer saved — queuing for processing");
                        if tx.send(ReplayBufferSaved { path: path_str.into() }).await.is_err() {
                            warn!("Daemon receiver dropped — shutting down OBS listener");
                            return Ok(());
                        }
                    } else {
                        warn!("ReplayBufferSaved event missing savedReplayPath field — raw event: {parsed}");
                    }
                }
            }

            op => {
                debug!(op, "Unhandled OBS op");
            }
        }
    }

    Ok(())
}
