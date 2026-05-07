use anyhow::Result;
use std::path::Path;
use tracing_appender::{non_blocking::WorkerGuard, rolling::{RollingFileAppender, Rotation}};
use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialise logging to both stderr and a daily-rotating log file.
///
/// The returned `WorkerGuard` must be held for the lifetime of the process;
/// dropping it flushes and closes the file sink.
pub fn init(log_dir: &Path) -> Result<WorkerGuard> {
    std::fs::create_dir_all(log_dir)?;

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("capture")
        .filename_suffix("log")
        .max_log_files(7)
        .build(log_dir)?;

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // File layer: full detail, no ANSI codes
    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(false)
        .with_writer(non_blocking)
        .with_filter(
            EnvFilter::from_default_env()
                .add_directive("capture=debug".parse()?)
                .add_directive("warn".parse()?),
        );

    // Stderr layer: compact, coloured, no module paths
    let stderr_layer = fmt::layer()
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_filter(
            EnvFilter::from_default_env()
                .add_directive("capture=info".parse()?)
                .add_directive("warn".parse()?),
        );

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .init();

    Ok(guard)
}
