//! Structured logs go to a rotating file, never to the TUI.

use std::io;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use crate::paths::AppPaths;

/// Installs a file subscriber. Keep the returned guard alive for the process lifetime.
pub fn init(paths: &AppPaths, debug: bool) -> Result<WorkerGuard, io::Error> {
    paths.ensure_dirs().map_err(io::Error::other)?;
    let appender = tracing_appender::rolling::daily(paths.log_dir(), "os-int.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let default = if debug {
        "os_int=debug,info"
    } else {
        "os_int=info,warn"
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_writer(writer),
        )
        .init();
    Ok(guard)
}
