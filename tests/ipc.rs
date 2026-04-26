use parking_lot::RwLock;
use pulsar_daemon::format;
use pulsar_daemon::ipc;
use pulsar_daemon::state::{
    BatteryPercent, ChargeState, DeviceState, DisconnectReason, LedState, LiftOffDistance,
    PollingRate, Power, Settings, Snapshot,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;

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

fn make_disconnected_state() -> DeviceState {
    DeviceState::Disconnected {
        since: Instant::now(),
        reason: DisconnectReason::Unplugged,
    }
}

#[test]
fn test_waybar_format() {
    let connected = make_connected_state();
    let json_connected = format::waybar(&connected);
    assert_eq!(
        json_connected.to_string(),
        r#"{"class":"normal","percentage":73,"text":"73%","tooltip":"Battery 73%\nVoltage 4.012V\nDischarging"}"#
    );

    let disconnected = make_disconnected_state();
    let json_disconnected = format::waybar(&disconnected);
    assert_eq!(
        json_disconnected.to_string(),
        r#"{"class":"critical","text":"Disconnected"}"#
    );
}

#[tokio::test]
async fn test_ipc_serve() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let socket_path = temp_dir.path().join("pulsar-x2.sock");

    let state = Arc::new(RwLock::new(make_connected_state()));
    let state_clone = state.clone();
    let socket_path_clone = socket_path.clone();

    // Spawn IPC server
    tokio::spawn(async move {
        let _ = ipc::serve(state_clone, socket_path_clone).await;
    });

    // Give it a moment to bind
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Connect and read connected state
    let mut stream = UnixStream::connect(&socket_path).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    assert_eq!(
        response,
        "{\"class\":\"normal\",\"percentage\":73,\"text\":\"73%\",\"tooltip\":\"Battery 73%\\nVoltage 4.012V\\nDischarging\"}\n"
    );

    // Change state to disconnected
    *state.write() = make_disconnected_state();

    // Reconnect and read disconnected state
    let mut stream = UnixStream::connect(&socket_path).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    assert_eq!(
        response,
        "{\"class\":\"critical\",\"text\":\"Disconnected\"}\n"
    );

    Ok(())
}
