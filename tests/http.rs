use parking_lot::RwLock;
use pulsar_daemon::format;
use pulsar_daemon::http;
use pulsar_daemon::state::{
    BatteryPercent, ChargeState, DeviceState, LedState, LiftOffDistance, PollingRate, Power,
    Settings, Snapshot,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;

fn make_connected_state() -> DeviceState {
    DeviceState::Connected {
        snapshot: Snapshot {
            power: Power {
                percent: BatteryPercent::new(73),
                charge: ChargeState::Discharging,
                voltage_mv: 4012,
            },
            settings: Settings {
                polling: PollingRate::Hz1000,
                dpi_slot: 1,
                dpi_slot_count: 4,
                lift_off: LiftOffDistance::Mm2,
                debounce_ms: 4,
                auto_sleep_seconds: 60,
                motion_sync: true,
                angle_snapping: false,
                lod_ripple: true,
                led: LedState::Steady,
            },
            profile: Some(1),
            settings_last_read: Instant::now(),
        },
        last_polled: Instant::now(),
    }
}

async fn get_http(addr: &str, path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(addr).await?;
    let request = format!("GET {} HTTP/1.0\r\nHost: {}\r\n\r\n", path, addr);
    stream.write_all(request.as_bytes()).await?;

    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    Ok(response)
}

#[tokio::test]
async fn test_http_endpoints() -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(RwLock::new(make_connected_state()));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let state_clone = state.clone();

    // Bind to 0 lets OS pick port, but we need to know it.
    // However, our `serve` fn takes a String and binds inside, not returning the port.
    // For test simplicity, we'll pick a non-standard port that's likely free.
    let bind_addr = "127.0.0.1:3132".to_string();

    tokio::spawn(async move {
        let _ = http::serve(state_clone, bind_addr, shutdown_rx).await;
    });

    // Give it a moment to bind
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Test /state
    let res = get_http("127.0.0.1:3132", "/state").await?;
    assert!(res.contains("HTTP/1.0 200 OK") || res.contains("HTTP/1.1 200 OK"));
    assert!(res.contains(r#""percent":73"#));

    // Test /waybar
    let res = get_http("127.0.0.1:3132", "/waybar").await?;
    assert!(res.contains("HTTP/1.0 200 OK") || res.contains("HTTP/1.1 200 OK"));

    // The body should match the format::waybar exact string
    let parts: Vec<&str> = res.split("\r\n\r\n").collect();
    let body = match parts.get(1) {
        Some(b) => b,
        None => return Err("No body found".into()),
    };
    let expected_waybar = format::waybar(&state.read()).to_string();
    assert_eq!(body.trim(), expected_waybar.trim());

    // Test / (Dashboard)
    let res = get_http("127.0.0.1:3132", "/").await?;
    assert!(res.contains("HTTP/1.0 200 OK") || res.contains("HTTP/1.1 200 OK"));
    assert!(res.contains("text/html"));
    assert!(res.contains("Pulsar X2 Status"));

    let _ = shutdown_tx.send(());
    Ok(())
}
