use crate::{
    config::Config,
    job::{CompressCodec, CompressOptions, Effect, Job, JobKind, JobStatus},
    queue::Queue,
    share as share_mod,
};
use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::Mutex,
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Holds the handle to the currently running ffmpeg process so it can be killed on cancel.
#[derive(Default)]
pub struct ProcessHandle {
    pub child: Option<tokio::process::Child>,
}

pub type SharedHandle = Arc<Mutex<ProcessHandle>>;

pub async fn run_worker(queue: Arc<Queue>, config: Arc<Config>) {
    info!("Processor worker started");
    loop {
        match queue.pop_next().await {
            None => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
            Some(job) => {
                let id = job.id;
                info!(
                    job_id = %id,
                    kind = %job.kind_label(),
                    source = %job.source.display(),
                    "Job started"
                );

                let handle: SharedHandle = Arc::new(Mutex::new(ProcessHandle::default()));
                let result = process_job(&job, &config, handle.clone(), queue.clone()).await;

                queue
                    .update(id, |j| {
                        if j.status != JobStatus::Cancelled {
                            j.status = match &result {
                                Ok(()) => JobStatus::Done,
                                Err(e) => JobStatus::Failed(e.to_string()),
                            };
                        }
                        j.progress = 100;
                        j.finished_at = Some(Utc::now());
                    })
                    .await;

                match &result {
                    Ok(()) => info!(job_id = %id, "Job completed successfully"),
                    Err(e) => {
                        error!(job_id = %id, error = %e, "Job failed");
                        cleanup_partial_output(&job);
                    }
                }
            }
        }
    }
}

async fn process_job(
    job: &Job,
    config: &Config,
    handle: SharedHandle,
    queue: Arc<Queue>,
) -> Result<()> {
    match &job.kind {
        JobKind::Upscale => upscale(job, config, handle, queue).await,
        JobKind::PostProcess { effects } => post_process(job, effects, handle, queue).await,
        JobKind::Compress(opts) => compress(job, opts, handle, queue).await,
        JobKind::Share { expiry } => share(job, config, queue, expiry).await,
    }
}

// ── Upscale ──────────────────────────────────────────────────────────────────

async fn upscale(
    job: &Job,
    config: &Config,
    handle: SharedHandle,
    queue: Arc<Queue>,
) -> Result<()> {
    let [width, height] = parse_resolution(&config.upscale_resolution)?;
    let resolution = format!("{width}:{height}");
    let encoder = &config.encoder;
    let cq = config.nvenc_quality.to_string();

    // Raw Lanczos pass — no colour grading here
    let vf = format!("scale={}:flags={}", resolution, config.upscale_filter);
    info!(job_id = %job.id, vf = %vf, "Video filter chain");

    let mut args = vec![
        "-y".to_string(),
        "-hwaccel".to_string(), "auto".to_string(),
        "-i".to_string(), job.source.to_string_lossy().into_owned(),
        "-vf".to_string(), vf,
        "-c:v".to_string(), encoder.clone(),
        "-c:a".to_string(), "copy".to_string(),
    ];

    if encoder.contains("nvenc") {
        args.extend(["-cq".to_string(), cq, "-preset".to_string(), "p4".to_string()]);
    } else {
        args.extend(["-crf".to_string(), cq]);
    }

    args.push(job.output.to_string_lossy().into_owned());
    run_ffmpeg_job(args, job.id, handle, queue).await
}

// ── PostProcess ───────────────────────────────────────────────────────────────

async fn post_process(
    job: &Job,
    effects: &[Effect],
    handle: SharedHandle,
    queue: Arc<Queue>,
) -> Result<()> {
    let vf = build_effects_vf(effects);
    info!(job_id = %job.id, vf = %vf, "Post-process filter chain");

    // CPU-side post-processing — no hwaccel, keep source resolution
    let mut args = vec![
        "-y".to_string(),
        "-i".to_string(), job.source.to_string_lossy().into_owned(),
    ];

    if !vf.is_empty() {
        args.extend(["-vf".to_string(), vf]);
    }

    // Re-encode video with high-quality h264 NVENC; copy audio
    args.extend([
        "-c:v".to_string(), "h264_nvenc".to_string(),
        "-cq".to_string(), "18".to_string(),
        "-preset".to_string(), "p4".to_string(),
        "-c:a".to_string(), "copy".to_string(),
    ]);

    args.push(job.output.to_string_lossy().into_owned());
    run_ffmpeg_job(args, job.id, handle, queue).await
}

/// Build a -vf filter string from an ordered list of `Effect`s.
fn build_effects_vf(effects: &[Effect]) -> String {
    let mut filters: Vec<String> = Vec::new();

    // Collect interpolation target fps (used for motion blur too)
    let interp_fps = effects.iter().find_map(|e| {
        if let Effect::Interpolate { target_fps } = e {
            Some(*target_fps)
        } else {
            None
        }
    });

    let motion_blur = effects.iter().find_map(|e| {
        if let Effect::MotionBlur { shutter_angle } = e {
            Some(*shutter_angle)
        } else {
            None
        }
    });

    // If motion blur is requested, we need high-fps intermediate frames.
    // Use the interpolation target if set, otherwise 120fps as the blur base.
    if let Some(angle) = motion_blur {
        let blur_fps = interp_fps.unwrap_or(120);
        // 1. Interpolate to high fps
        filters.push(format!(
            "minterpolate=fps={blur_fps}:mi_mode=mci:mc_mode=aobmc:vsbmc=1"
        ));
        // 2. Blend frames — shutter_angle/360 is the fraction of the frame period to blend.
        //    E.g. 180° → blend ~half the frames, 360° → blend all.
        let blend_frames = ((angle as f32 / 360.0) * (blur_fps as f32 / 60.0)).round() as u32;
        let blend_frames = blend_frames.max(2);
        filters.push(format!("tmix=frames={blend_frames}:weights=1"));
        // 3. If the user also asked for a specific interpolated output fps, keep it.
        //    Otherwise downsample back to ~60 fps.
        let output_fps = interp_fps.unwrap_or(60);
        filters.push(format!("fps={output_fps}"));
    } else if let Some(fps) = interp_fps {
        // Frame interpolation only — no blur
        filters.push(format!(
            "minterpolate=fps={fps}:mi_mode=mci:mc_mode=aobmc:vsbmc=1"
        ));
    }

    for effect in effects {
        match effect {
            Effect::Saturation(v) if (*v - 1.0).abs() > 0.01 => {
                filters.push(format!("eq=saturation={v:.2}"));
            }
            Effect::Sharpen(v) if *v > 0.0 => {
                filters.push(format!("unsharp=5:5:{v:.2}:5:5:0.0"));
            }
            // Interpolate and MotionBlur handled above
            _ => {}
        }
    }

    filters.join(",")
}

// ── Compress ──────────────────────────────────────────────────────────────────

async fn compress(
    job: &Job,
    opts: &CompressOptions,
    handle: SharedHandle,
    queue: Arc<Queue>,
) -> Result<()> {
    let codec = opts.codec.ffmpeg_codec();
    let crf = opts.quality.crf(&opts.codec).to_string();

    let mut args = vec!["-y".to_string()];

    // Trim seek (input-side for fast seek)
    if let Some(start) = &opts.trim_start {
        args.extend(["-ss".to_string(), start.clone()]);
    }

    args.extend([
        "-i".to_string(), job.source.to_string_lossy().into_owned(),
    ]);

    // Output trim end
    if let Some(end) = &opts.trim_end {
        args.extend(["-to".to_string(), end.clone()]);
    }

    args.extend([
        "-c:v".to_string(), codec.to_string(),
        "-c:a".to_string(), "copy".to_string(),
    ]);

    if opts.codec.is_nvenc() {
        args.extend([
            "-cq".to_string(), crf,
            "-preset".to_string(), "p4".to_string(),
        ]);
    } else if matches!(opts.codec, CompressCodec::Av1) {
        args.extend([
            "-crf".to_string(), crf,
            "-b:v".to_string(), "0".to_string(),
            "-cpu-used".to_string(), "4".to_string(),
        ]);
    } else {
        // libx265
        args.extend([
            "-crf".to_string(), crf,
            "-preset".to_string(), "medium".to_string(),
        ]);
    }

    args.push(job.output.to_string_lossy().into_owned());

    info!(
        job_id = %job.id,
        codec = %opts.codec.label(),
        quality = %opts.quality.label(),
        command = %format!("ffmpeg {}", args.join(" ")),
        "Spawning compress ffmpeg"
    );

    run_ffmpeg_job(args, job.id, handle, queue).await
}

// ── Share ─────────────────────────────────────────────────────────────────────

async fn share(job: &Job, _config: &Config, queue: Arc<Queue>, expiry: &str) -> Result<()> {
    info!(job_id = %job.id, path = %job.source.display(), expiry, "Uploading to litterbox");

    queue.update(job.id, |j| j.progress = 5).await;

    let url = share_mod::upload(&job.source, expiry).await?;

    info!(job_id = %job.id, url = %url, "Upload complete");

    // Persist URL to shares.json (no deletion token with litterbox)
    let data_dir = Config::data_dir();
    let mut store = crate::clips::ShareStore::load(&data_dir);
    store.set(&job.source, url.clone(), String::new());
    if let Err(e) = store.save(&data_dir) {
        warn!(error = %e, "Failed to persist share URL");
    }

    let url_clone = url.clone();
    queue
        .update(job.id, |j| {
            j.share_url = Some(url_clone);
            j.share_token = None;
        })
        .await;

    Ok(())
}

// ── ffmpeg runner ─────────────────────────────────────────────────────────────

async fn run_ffmpeg_job(
    args: Vec<String>,
    job_id: Uuid,
    handle: SharedHandle,
    queue: Arc<Queue>,
) -> Result<()> {
    std::fs::create_dir_all(
        // args.last() is the output path
        args.last()
            .and_then(|p| std::path::Path::new(p).parent())
            .unwrap_or(std::path::Path::new(".")),
    )?;

    info!(
        job_id = %job_id,
        command = %format!("ffmpeg {}", args.join(" ")),
        "Spawning ffmpeg"
    );

    #[allow(unused_mut)]
    let mut cmd = Command::new("ffmpeg");
    cmd.args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn()?;

    let stderr = child.stderr.take().unwrap();
    handle.lock().await.child = Some(child);

    let mut reader = BufReader::new(stderr).lines();
    let mut duration_secs: Option<f64> = None;
    let mut last_pct: Option<u8> = None;
    let mut last_logged_pct: u8 = 0;

    while let Ok(Some(line)) = reader.next_line().await {
        // Check cancellation once per line
        let snapshot = queue.snapshot().await;
        if let Some(j) = snapshot.iter().find(|j| j.id == job_id) {
            if j.status == JobStatus::Cancelled {
                info!(job_id = %job_id, "Cancellation requested — killing ffmpeg");
                if let Some(child) = handle.lock().await.child.as_mut() {
                    let _ = child.kill().await;
                }
                cleanup_ffmpeg_output(&args);
                return Ok(());
            }
        }

        // Windows ffmpeg uses \r (not \n) for progress updates, so a single
        // \n-terminated "line" may contain many \r-separated progress segments.
        for segment in line.split('\r') {
            debug!(job_id = %job_id, segment = %segment, "ffmpeg");

            if duration_secs.is_none() {
                if let Some(d) = parse_ffmpeg_duration(segment) {
                    duration_secs = Some(d);
                    info!(job_id = %job_id, duration_secs = d, "ffmpeg: clip duration parsed");
                }
            }

            if let (Some(total), Some(current)) = (duration_secs, parse_ffmpeg_time(segment)) {
                let pct = ((current / total) * 100.0).min(99.0) as u8;
                // Only broadcast when the integer percentage actually changes —
                // prevents flooding the broadcast channel with identical updates.
                if last_pct != Some(pct) {
                    last_pct = Some(pct);
                    queue.update(job_id, |j| j.progress = pct).await;

                    if pct >= last_logged_pct + 25 {
                        info!(job_id = %job_id, progress = pct, "ffmpeg progress");
                        last_logged_pct = pct;
                    }
                }
            }

            if segment.contains("Error") || segment.contains("Invalid") || segment.contains("No such file") {
                warn!(job_id = %job_id, segment = %segment, "ffmpeg warning/error line");
            }
        }
    }

    let status = if let Some(child) = handle.lock().await.child.as_mut() {
        child.wait().await?
    } else {
        return Ok(());
    };

    if !status.success() {
        anyhow::bail!("ffmpeg exited with code {}", status.code().unwrap_or(-1));
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn cleanup_partial_output(job: &Job) {
    if job.output.exists() {
        match std::fs::remove_file(&job.output) {
            Ok(()) => info!(path = %job.output.display(), "Cleaned up partial output"),
            Err(e) => warn!(path = %job.output.display(), error = %e, "Failed to clean up partial output"),
        }
    }
}

fn cleanup_ffmpeg_output(args: &[String]) {
    if let Some(path) = args.last() {
        let p = std::path::Path::new(path);
        if p.exists() {
            match std::fs::remove_file(p) {
                Ok(()) => info!(path = %p.display(), "Cleaned up partial output after cancel"),
                Err(e) => warn!(path = %p.display(), error = %e, "Failed to clean up after cancel"),
            }
        }
    }
}

fn parse_resolution(s: &str) -> Result<[u32; 2]> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() != 2 {
        anyhow::bail!("invalid resolution: {s}");
    }
    Ok([parts[0].parse()?, parts[1].parse()?])
}

fn parse_ffmpeg_duration(line: &str) -> Option<f64> {
    let prefix = "  Duration: ";
    let pos = line.find(prefix)?;
    let rest = &line[pos + prefix.len()..];
    parse_hhmmss(rest.split(',').next()?)
}

fn parse_ffmpeg_time(line: &str) -> Option<f64> {
    let prefix = "time=";
    let pos = line.find(prefix)?;
    let rest = &line[pos + prefix.len()..];
    parse_hhmmss(rest.split_whitespace().next()?)
}

fn parse_hhmmss(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let s: f64 = parts[2].parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}
