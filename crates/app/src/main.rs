//! FarmController process composition and runtime lifecycle.
//!
//! This binary is the single entrypoint that replaces the separate Node
//! `auth-service` / `device-service` process starts. It loads config, boots
//! shared state, mounts the Axum app, starts background workers, and performs
//! graceful shutdown on Ctrl+C / SIGTERM.

use std::{future::IntoFuture, sync::Arc, time::Duration};

use clap::Parser;
use farmcontroller::{
    admin_control, build_app,
    cli::Cli,
    config::{AppConfig, Module},
    observability::{Metrics, init_logging},
    state::AppState,
    workers::types::WorkerManager,
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Maximum time to wait for in-flight HTTP connections after cancel.
const SERVER_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// PURPOSE: Bootstrap config, state, HTTP server, and workers; block until shutdown.
/// USE: OS process entry. Side effects: binds the listener, starts workers,
/// installs signal handlers, and tears everything down on cancel.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let config = admin_control::activate(AppConfig::load(&cli)?)?;
    let _log_guard = init_logging(&config.logging)?;

    let metrics = Metrics::new()?;
    let state = Arc::new(AppState::bootstrap(config.clone(), metrics).await?);
    let app = build_app(state.clone());
    let listener = TcpListener::bind(config.server.bind).await?;
    let cancellation = CancellationToken::new();
    let workers = WorkerManager::start(state.clone(), cancellation.child_token());

    info!(
        version = env!("CARGO_PKG_VERSION"),
        bind = %config.server.bind,
        modules = ?config.modules.enabled,
        "FarmController starting"
    );
    if config.module_enabled(Module::Swagger) {
        info!("Swagger available at /swagger-ui/");
    }
    if config.module_enabled(Module::Metrics) {
        info!("Prometheus metrics available at /metrics");
    }

    let signal_cancellation = cancellation.clone();
    let signal_task = tokio::spawn(async move {
        shutdown_signal().await;
        signal_cancellation.cancel();
    });
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(cancellation.clone().cancelled_owned())
        .into_future();
    tokio::pin!(server);
    // Prefer clean drain; after grace, abandon remaining connections.
    let server_result = tokio::select! {
        result = &mut server => result,
        _ = cancellation.cancelled() => {
            state.shutdown_events().await;
            match tokio::time::timeout(SERVER_SHUTDOWN_GRACE, &mut server).await {
                Ok(result) => result,
                Err(_) => {
                    warn!(grace_seconds = SERVER_SHUTDOWN_GRACE.as_secs(), "forcing server shutdown with connections still open");
                    Ok(())
                }
            }
        }
    };
    cancellation.cancel();
    workers.shutdown().await;
    signal_task.abort();
    server_result?;

    info!("FarmController stopped");
    Ok(())
}

/// PURPOSE: Wait for Ctrl+C or SIGTERM (Unix) before initiating shutdown.
/// USE: Spawned alongside the HTTP server; cancels the shared token when fired.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to install Ctrl+C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(%error, "failed to install SIGTERM handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!("shutdown signal received");
}
