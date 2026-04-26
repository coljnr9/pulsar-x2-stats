use std::path::PathBuf;
use std::process::Command;
use tokio::sync::watch;

pub async fn run(mut change_rx: watch::Receiver<u64>, pid_file: Option<PathBuf>, signal_num: u32) {
    tracing::info!("Waybar notifier started (signal SIGRTMIN+{})", signal_num);

    // Wait for the first change, skipping the initial value
    while change_rx.changed().await.is_ok() {
        // A change happened
        if let Some(pid) = resolve_waybar_pid(&pid_file) {
            // Using Command to avoid unsafe block while still sending RT signals
            // that nix::sys::signal::Signal doesn't support well.
            let sigrtmin = 34; // standard on Linux
            let target_sig = sigrtmin + signal_num;

            match Command::new("kill")
                .arg(format!("-{}", target_sig))
                .arg(pid.to_string())
                .status()
            {
                Ok(status) if status.success() => {
                    tracing::debug!("Sent signal {} to Waybar (PID {})", target_sig, pid);
                }
                Ok(status) => {
                    tracing::warn!(
                        "Failed to send signal to Waybar (PID {}): exit code {}",
                        pid,
                        status
                    );
                }
                Err(e) => {
                    tracing::warn!("Failed to execute kill command: {}", e);
                }
            }
        }
    }
}

#[allow(clippy::collapsible_if)]
fn resolve_waybar_pid(pid_file: &Option<PathBuf>) -> Option<i32> {
    if let Some(path) = pid_file {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(pid) = content.trim().parse::<i32>() {
                return Some(pid);
            }
        }
    }

    // Scan /proc
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    let file_name = entry.file_name();
                    let file_name_str = file_name.to_string_lossy();
                    if file_name_str.chars().all(|c| c.is_ascii_digit()) {
                        let mut comm_path = entry.path();
                        comm_path.push("comm");
                        if let Ok(comm) = std::fs::read_to_string(comm_path) {
                            if comm.trim() == "waybar" {
                                if let Ok(pid) = file_name_str.parse::<i32>() {
                                    return Some(pid);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}
