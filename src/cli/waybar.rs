use std::path::PathBuf;
use std::process::exit;
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;

pub async fn run(socket_path: Option<PathBuf>) {
    let path = match socket_path {
        Some(p) => p,
        None => match crate::ipc::get_default_socket_path() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{}", e);
                // Print a disconnected state for waybar so it doesn't break
                println!("{{\"text\":\"Disconnected\",\"class\":\"critical\"}}");
                exit(0);
            }
        },
    };

    match UnixStream::connect(&path).await {
        Ok(mut stream) => {
            let mut response = String::new();
            if let Err(e) = stream.read_to_string(&mut response).await {
                tracing::error!("Failed to read from IPC socket: {}", e);
                println!("{{\"text\":\"Disconnected\",\"class\":\"critical\"}}");
                exit(0);
            }
            // The daemon sends exactly one JSON line and closes the connection.
            print!("{}", response);
            exit(0);
        }
        Err(e) => {
            tracing::error!("Failed to connect to IPC socket at {:?}: {}", path, e);
            println!("{{\"text\":\"Disconnected\",\"class\":\"critical\"}}");
            exit(0);
        }
    }
}
