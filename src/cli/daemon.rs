use crate::device::DeviceWorker;
use crate::poll::{PollConfig, run as poll_run};
use crate::state::{DeviceState, DisconnectReason, StateBus};
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::oneshot;

use std::path::PathBuf;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    poll_interval_secs: u64,
    asleep_poll_interval_secs: u64,
    settings_refresh_every: u64,
    reconnect_backoff_max_secs: u64,
    socket_path: Option<PathBuf>,
    waybar_pid_file: Option<PathBuf>,
    waybar_signal: u32,
    http_bind: String,
) {
    let state = Arc::new(RwLock::new(DeviceState::Disconnected {
        since: Instant::now(),
        reason: DisconnectReason::NeverConnected,
    }));

    let (change_tx, change_rx) = tokio::sync::watch::channel(0);

    let bus = StateBus {
        state: state.clone(),
        change_tx,
    };

    let worker = DeviceWorker::spawn();

    let cfg = PollConfig {
        poll_interval: Duration::from_secs(poll_interval_secs),
        asleep_poll_interval: Duration::from_secs(asleep_poll_interval_secs),
        settings_refresh_every,
        backoff_max_secs: reconnect_backoff_max_secs,
    };

    tracing::info!("Starting daemon polling loop...");

    let poll_task = tokio::spawn(async move {
        if let Err(e) = poll_run(worker, bus, cfg).await {
            tracing::error!("Poll task exited with error: {}", e);
        }
    });

    let resolved_socket_path = match socket_path {
        Some(p) => p,
        None => match crate::ipc::get_default_socket_path() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to resolve socket path: {}", e);
                return;
            }
        },
    };

    let ipc_state = state.clone();
    let socket_path_to_remove = resolved_socket_path.clone();
    let ipc_task = tokio::spawn(async move {
        if let Err(e) = crate::ipc::serve(ipc_state, resolved_socket_path).await {
            tracing::error!("IPC server exited with error: {}", e);
        }
    });

    let notify_task = tokio::spawn(async move {
        crate::notify::run(change_rx, waybar_pid_file, waybar_signal).await;
    });

    let (http_shutdown_tx, http_shutdown_rx) = oneshot::channel();
    let http_state = state.clone();
    let http_task = tokio::spawn(async move {
        if let Err(e) = crate::http::serve(http_state, http_bind, http_shutdown_rx).await {
            tracing::error!("HTTP server exited with error: {}", e);
        }
    });

    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to register SIGTERM handler: {}", e);
            return;
        }
    };
    let mut sigint = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to register SIGINT handler: {}", e);
            return;
        }
    };

    tokio::select! {
        _ = poll_task => {
            tracing::warn!("Polling loop finished unexpectedly.");
        }
        _ = ipc_task => {
            tracing::warn!("IPC server finished unexpectedly.");
        }
        _ = notify_task => {
            tracing::warn!("Notifier finished unexpectedly.");
        }
        _ = http_task => {
            tracing::warn!("HTTP server finished unexpectedly.");
        }
        _ = sigterm.recv() => {
            tracing::info!("Received SIGTERM, shutting down.");
        }
        _ = sigint.recv() => {
            tracing::info!("Received SIGINT, shutting down.");
        }
    }

    let _ = http_shutdown_tx.send(());

    tracing::info!("Cleaning up IPC socket at {:?}", socket_path_to_remove);
    if socket_path_to_remove.exists() {
        if let Err(e) = std::fs::remove_file(&socket_path_to_remove) {
            tracing::error!("Failed to remove IPC socket: {}", e);
        } else {
            tracing::info!("IPC socket removed successfully.");
        }
    }
}
