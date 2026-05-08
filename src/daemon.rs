use crate::{
    clips::ClipStore,
    config::Config,
    ipc,
    job::{Job, JobKind, JobStatus},
    notify,
    obs,
    obs_profile,
    processor,
    queue::{Queue, QueueEvent},
    updater,
};
use anyhow::Result;
use std::{path::PathBuf, sync::Arc};
use tracing::{error, info, warn};

pub async fn run(config: Config) -> Result<()> {
    let config = Arc::new(config);

    // Clip library — load from disk, then watch for changes
    let clips = ClipStore::new(
        Config::data_dir(),
        PathBuf::from(&config.output_dir),
    );
    clips.init().await;
    info!(output_dir = %config.output_dir, "Clip library initialised");

    let queue = Queue::new();

    // ── Update check (background, non-blocking) ───────────────────────────────
    let (update_tx, update_rx) = tokio::sync::watch::channel::<Option<String>>(None);
    let update_tx = Arc::new(update_tx);
    {
        let tx = update_tx.clone();
        tokio::spawn(async move {
            match updater::check_latest().await {
                Ok(Some(rel)) => {
                    info!(version = %rel.version, "Update available");
                    let _ = tx.send(Some(rel.version));
                }
                Ok(None) => info!("rscapt is up to date"),
                Err(e)   => warn!(error = %e, "Update check failed"),
            }
        });
    }

    info!(ipc_port = config.ipc_port, "Starting IPC server");
    {
        let q = queue.clone();
        let c = clips.clone();
        let port = config.ipc_port;
        let urx = update_rx;
        tokio::spawn(async move {
            if let Err(e) = ipc::serve(port, q, c, urx).await {
                error!("IPC server died: {e}");
            }
        });
    }

    info!(encoder = %config.encoder, output_dir = %config.output_dir, "Starting job processor");
    {
        let q = queue.clone();
        let cfg = config.clone();
        tokio::spawn(async move {
            processor::run_worker(q, cfg).await;
        });
    }

    // Watch job completions → toasts + clip library updates
    {
        let mut events = queue.subscribe();
        let clips_watch = clips.clone();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(QueueEvent::JobUpdated(job)) => on_job_event(&job, &clips_watch).await,
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(n, "daemon event loop lagged — some progress updates skipped");
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // OBS launch: try connecting first (via obs::run's retry loop).
    // If we manage OBS and it isn't already up after 2 s, launch it.
    // The WebSocket listener handles reconnection so no blocking wait needed.
    if config.obs_managed && !config.obs_exe_path.is_empty() {
        let exe = PathBuf::from(&config.obs_exe_path);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if obs_profile::is_running() {
                info!("OBS already running — skipping managed launch");
            } else {
                match obs_profile::launch(&exe) {
                    Ok(()) => info!(path = %exe.display(), "Launched managed OBS"),
                    Err(e) => warn!(error = %e, "Failed to launch managed OBS"),
                }
            }
        });
    }

    info!(obs = %format!("{}:{}", config.obs_host, config.obs_port), "Starting OBS listener");
    let (obs_tx, mut obs_rx) = tokio::sync::mpsc::channel(16);
    {
        let cfg = config.clone();
        tokio::spawn(async move {
            if let Err(e) = obs::run(cfg, obs_tx).await {
                error!("OBS listener exited unexpectedly: {e}");
            }
        });
    }

    // Keep update_tx alive so watch receivers stay valid for the daemon lifetime
    let _update_tx = update_tx;

    info!("Daemon ready — waiting for replay buffer saves");

    while let Some(saved) = obs_rx.recv().await {
        let output = build_output_path(&saved.path, &config.output_dir);
        info!(
            source = %saved.path.display(),
            output = %output.display(),
            "Queuing upscale job"
        );
        let job = Job::new(JobKind::Upscale, saved.path, output);
        queue.push(job).await;
    }

    error!("OBS channel closed — daemon shutting down");
    Ok(())
}

async fn on_job_event(job: &Job, clips: &Arc<ClipStore>) {
    match &job.status {
        JobStatus::Done => {
            match &job.kind {
                JobKind::Upscale | JobKind::PostProcess { .. } | JobKind::Compress(_) => {
                    clips.add_if_new(&job.output).await;
                    notify::toast(
                        &format!("{} done", job.kind_label()),
                        &format!(
                            "{} → {}",
                            job.display_name(),
                            job.output.file_name().unwrap_or_default().to_string_lossy()
                        ),
                    );
                }
                JobKind::Share { .. } => {
                    if let Some(url) = &job.share_url {
                        clips.set_share(&job.source, url.clone(), String::new()).await;
                        notify::toast("Clip shared", url);
                    }
                }
            }

            info!(
                job_id = %job.id,
                kind = %job.kind_label(),
                "Job done"
            );
        }
        JobStatus::Failed(e) => {
            error!(job_id = %job.id, kind = %job.kind_label(), error = %e, "Job failed");
            notify::toast(
                &format!("{} failed", job.kind_label()),
                &format!("{}: {e}", job.display_name()),
            );
        }
        JobStatus::Cancelled => {
            info!(job_id = %job.id, kind = %job.kind_label(), "Job cancelled");
        }
        _ => {}
    }
}

fn build_output_path(source: &PathBuf, output_dir: &str) -> PathBuf {
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let ext = source.extension().unwrap_or_default().to_string_lossy();
    PathBuf::from(output_dir).join(format!("{stem}_1440p.{ext}"))
}
