use crate::poll::DaemonError;
use crate::state::SharedState;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;

pub async fn serve(state: SharedState, socket_path: PathBuf) -> Result<(), DaemonError> {
    if socket_path.exists() {
        // Try connecting to see if another daemon is running
        match tokio::net::UnixStream::connect(&socket_path).await {
            Ok(_) => {
                return Err(DaemonError::Poll(
                    "Another daemon instance is already running (socket connects)".to_string(),
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                // Stale socket, remove it
                if let Err(e) = std::fs::remove_file(&socket_path) {
                    return Err(DaemonError::Poll(format!(
                        "Failed to remove stale socket: {}",
                        e
                    )));
                }
            }
            Err(e) => {
                return Err(DaemonError::Poll(format!(
                    "Failed to check existing socket: {}",
                    e
                )));
            }
        }
    } else if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let listener = UnixListener::bind(&socket_path)
        .map_err(|e| DaemonError::Poll(format!("Failed to bind socket: {}", e)))?;

    // chmod 0o600
    let mut perms = std::fs::metadata(&socket_path)
        .map_err(|e| DaemonError::Poll(format!("Failed to get socket metadata: {}", e)))?
        .permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(&socket_path, perms)
        .map_err(|e| DaemonError::Poll(format!("Failed to set socket permissions: {}", e)))?;

    tracing::info!("IPC socket listening on {:?}", socket_path);

    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                let current_state = state.read().clone();
                let waybar_json = crate::format::waybar(&current_state);
                let line = format!("{}\n", waybar_json);
                let _ = stream.write_all(line.as_bytes()).await;
                // close implies drop
            }
            Err(e) => {
                tracing::error!("IPC accept error: {}", e);
            }
        }
    }
}

pub fn get_default_socket_path() -> Result<PathBuf, String> {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(runtime_dir).join("pulsar-x2.sock"));
    }

    // Fallback: /run/user/$UID
    let uid = nix::unistd::Uid::current().as_raw();
    let path = PathBuf::from(format!("/run/user/{}/pulsar-x2.sock", uid));
    if path.parent().is_some_and(|p| p.exists()) {
        return Ok(path);
    }

    Err(
        "Could not determine a safe socket path. Set XDG_RUNTIME_DIR or specify --socket-path."
            .to_string(),
    )
}
