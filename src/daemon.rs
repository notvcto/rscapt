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

    info!(ipc_port = config.ipc_port, "Starting IPC server");
    {
        let q = queue.clone();
        let c = clips.clone();
        let port = config.ipc_port;
        tokio::spawn(async move {
            if let Err(e) = ipc::serve(port, q, c).await {
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
            while let Ok(event) = events.recv().await {
                if let QueueEvent::JobUpdated(job) = event {
                    on_job_event(&job, &clips_watch).await;
                }
            }
        });
    }

    // Launch OBS if we own it and it isn't already running
    if config.obs_managed && !config.obs_exe_path.is_empty() {
        if obs_profile::is_running() {
            info!("OBS already running — skipping launch");
        } else {
            let exe = PathBuf::from(&config.obs_exe_path);
            match obs_profile::launch(&exe) {
                Ok(()) => info!(path = %exe.display(), "Launched OBS"),
                Err(e) => warn!(error = %e, "Failed to launch OBS — continuing anyway"),
            }
            // Brief pause to let OBS initialise its WebSocket server
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
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
                    // Register the output file in the clip library
                    clips.add_if_new(&job.output).await;

                    notify::toast(
                        &format!("{} done", job.kind_label()),
                        &format!(
                            "{} → {}",
                            job.display_name(),
                            job.output
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                        ),
                    );
                }
                JobKind::Share => {
                    // Share job stores url/token on the job; propagate to clip library
                    if let (Some(url), Some(token)) = (&job.share_url, &job.share_token) {
                        clips.set_share(&job.source, url.clone(), token.clone()).await;
                        notify::toast(
                            "Clip shared",
                            url,
                        );
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
