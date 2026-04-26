use clap::Parser;
use tracing_subscriber::EnvFilter;

use pulsar_daemon::cli;
use pulsar_daemon::poll;
use pulsar_daemon::protocol;
use pulsar_daemon::transport;

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("transport: {0}")]
    Transport(#[from] transport::TransportError),
    #[error("protocol: {0}")]
    Protocol(#[from] protocol::ParseError),
    #[error("daemon: {0}")]
    Daemon(#[from] poll::DaemonError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Parser, Debug)]
#[command(
    name = "pulsar-daemon",
    version = "0.1.0",
    author,
    about = "Pulsar X2 Rust Daemon"
)]
struct Cli {
    #[command(subcommand)]
    command: cli::CliCommand,
}

fn main() -> Result<(), AppError> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Cli::parse();

    match args.command {
        cli::CliCommand::ReadPower => {
            cli::read_power::run();
            Ok(())
        }
        cli::CliCommand::Daemon {
            poll_interval_secs,
            asleep_poll_interval_secs,
            settings_refresh_every,
            reconnect_backoff_max_secs,
            socket_path,
            waybar_pid_file,
            waybar_signal,
            http_bind,
        } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;

            rt.block_on(cli::daemon::run(
                poll_interval_secs,
                asleep_poll_interval_secs,
                settings_refresh_every,
                reconnect_backoff_max_secs,
                socket_path,
                waybar_pid_file,
                waybar_signal,
                http_bind,
            ));
            Ok(())
        }
        cli::CliCommand::Waybar { socket_path } => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(cli::waybar::run(socket_path));
            Ok(())
        }
    }
}
