pub mod daemon;
pub mod read_power;
pub mod waybar;

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum CliCommand {
    Daemon {
        #[arg(long, default_value = "10")]
        poll_interval_secs: u64,

        #[arg(long, default_value = "60")]
        asleep_poll_interval_secs: u64,

        #[arg(long, default_value = "6")]
        settings_refresh_every: u64,

        #[arg(long)]
        socket_path: Option<std::path::PathBuf>,

        #[arg(long, default_value = "127.0.0.1:3131")]
        http_bind: String,

        #[arg(long)]
        waybar_pid_file: Option<std::path::PathBuf>,

        #[arg(long, default_value = "8")]
        waybar_signal: u32,

        #[arg(long, default_value = "30")]
        reconnect_backoff_max_secs: u64,
    },
    Waybar {
        #[arg(long)]
        socket_path: Option<std::path::PathBuf>,
    },
    ReadPower,
}
