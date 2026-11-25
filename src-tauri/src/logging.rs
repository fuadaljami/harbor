use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::EnvFilter;

use crate::error::{AppError, AppResult};

const MAX_RETAINED_LOGS: usize = 7;

/// Keeps the background log writer alive until application shutdown.
pub struct LogGuard {
    _worker_guard: WorkerGuard,
}

/// Initializes daily rotating disk logs inside `logs_dir` (and stderr in debug builds).
pub fn init_logging(logs_dir: &Path) -> AppResult<LogGuard> {
    std::fs::create_dir_all(logs_dir)?;
    prune_old_logs(logs_dir, MAX_RETAINED_LOGS);

    let file_appender = tracing_appender::rolling::daily(logs_dir, "harbor.log");
    let (non_blocking, worker_guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("harbor=info,harbor_lib=info,warn"));

    let init_result = if cfg!(debug_assertions) {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_target(true)
            .with_writer(non_blocking.and(std::io::stderr))
            .try_init()
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_target(true)
            .with_writer(non_blocking)
            .try_init()
    };

    if let Err(err) = init_result {
        // Ignore if a global subscriber was already set in the same process (e.g., tests).
        let msg = err.to_string();
        if !msg.contains("already been set") {
            return Err(AppError::Config(format!(
                "Failed to initialize logger: {msg}"
            )));
        }
    }

    Ok(LogGuard {
        _worker_guard: worker_guard,
    })
}

fn prune_old_logs(logs_dir: &Path, retain_count: usize) {
    let Ok(entries) = std::fs::read_dir(logs_dir) else {
        return;
    };

    let mut log_files: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.starts_with("harbor.log") {
                return None;
            }
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            Some((modified, path))
        })
        .collect();

    if log_files.len() <= retain_count {
        return;
    }

    log_files.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, old_path) in log_files.into_iter().skip(retain_count) {
        let _ = std::fs::remove_file(old_path);
    }
}
