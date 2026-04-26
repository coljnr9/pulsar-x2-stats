use parking_lot::RwLock;
use pulsar_daemon::device::{DeviceWorker, HidCommand};
use pulsar_daemon::poll::{PollConfig, PollError, PollOutcome, try_poll_cycle};
use pulsar_daemon::state::{
    BatteryPercent, ChargeState, DeviceState, DisconnectReason, LedState, LiftOffDistance,
    PollingRate, Power, Settings, StateBus,
};
use pulsar_daemon::transport::TransportError;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

fn default_settings() -> Settings {
    Settings {
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
    }
}

fn default_power() -> Power {
    Power {
        percent: BatteryPercent::new(100),
        charge: ChargeState::Discharging,
        voltage_mv: 4100,
    }
}

struct MockDeviceRunner {
    pub rx: mpsc::Receiver<HidCommand>,
    pub power_res: Result<Power, TransportError>,
    pub settings_res: Result<Settings, TransportError>,
    pub is_present: bool,
}

impl MockDeviceRunner {
    async fn step(&mut self) {
        if let Some(cmd) = self.rx.recv().await {
            match cmd {
                HidCommand::GetPower(tx) => {
                    let _ = tx.send(self.power_res.as_ref().map(|p| p.clone()).map_err(
                        |e| match e {
                            TransportError::Timeout(d) => TransportError::Timeout(*d),
                            _ => TransportError::NotFound {
                                vendor: 0,
                                product: 0,
                            },
                        },
                    ));
                }
                HidCommand::ReadSettings(tx) => {
                    let _ = tx.send(self.settings_res.as_ref().map(|s| s.clone()).map_err(|_| {
                        TransportError::NotFound {
                            vendor: 0,
                            product: 0,
                        }
                    }));
                }
                HidCommand::GetActiveProfile(tx) => {
                    let _ = tx.send(Ok(0));
                }
                HidCommand::IsPresent(tx) => {
                    let _ = tx.send(self.is_present);
                }
                HidCommand::Shutdown => {}
            }
        }
    }
}

#[tokio::test]
async fn test_state_machine() -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(RwLock::new(DeviceState::Disconnected {
        since: Instant::now(),
        reason: DisconnectReason::NeverConnected,
    }));
    let (change_tx, mut change_rx) = tokio::sync::watch::channel(0);
    let bus = StateBus {
        state: state.clone(),
        change_tx,
    };
    let cfg = PollConfig {
        poll_interval: Duration::from_secs(1),
        asleep_poll_interval: Duration::from_secs(60),
        settings_refresh_every: 6,
        backoff_max_secs: 30,
    };

    let (tx, rx) = mpsc::channel(32);
    let worker = DeviceWorker::spawn_mock(tx);
    let mut runner = MockDeviceRunner {
        rx,
        power_res: Ok(default_power()),
        settings_res: Ok(default_settings()),
        is_present: true,
    };

    let mut tick = 0;

    let _runner_handle = tokio::spawn(async move {
        loop {
            runner.step().await;
        }
    });

    // 1. Initial success -> Connected
    let outcome1 = try_poll_cycle(&worker, &bus, &mut tick, &cfg).await;
    assert!(matches!(outcome1, Ok(PollOutcome::Connected)));
    assert!(matches!(*bus.state.read(), DeviceState::Connected { .. }));
    let _ = change_rx.changed().await;

    // 2. Timeout but present -> Asleep
    let (tx, rx) = mpsc::channel(32);
    let worker = DeviceWorker::spawn_mock(tx);
    let mut runner = MockDeviceRunner {
        rx,
        power_res: Err(TransportError::Timeout(Duration::from_secs(1))),
        settings_res: Ok(default_settings()),
        is_present: true,
    };
    let _runner_handle = tokio::spawn(async move {
        loop {
            runner.step().await;
        }
    });

    let outcome2 = try_poll_cycle(&worker, &bus, &mut tick, &cfg).await;
    assert!(matches!(outcome2, Ok(PollOutcome::Asleep)));
    assert!(matches!(*bus.state.read(), DeviceState::Asleep { .. }));
    let _ = change_rx.changed().await;

    // 3. Another success -> Connected
    let (tx, rx) = mpsc::channel(32);
    let worker = DeviceWorker::spawn_mock(tx);
    let mut runner = MockDeviceRunner {
        rx,
        power_res: Ok(default_power()),
        settings_res: Ok(default_settings()),
        is_present: true,
    };
    let _runner_handle = tokio::spawn(async move {
        loop {
            runner.step().await;
        }
    });

    let outcome3 = try_poll_cycle(&worker, &bus, &mut tick, &cfg).await;
    assert!(matches!(outcome3, Ok(PollOutcome::Connected)));
    assert!(matches!(*bus.state.read(), DeviceState::Connected { .. }));
    let _ = change_rx.changed().await;

    // 4. Not present -> Disconnected
    let (tx, rx) = mpsc::channel(32);
    let worker = DeviceWorker::spawn_mock(tx);
    let mut runner = MockDeviceRunner {
        rx,
        power_res: Err(TransportError::Timeout(Duration::from_secs(1))),
        settings_res: Ok(default_settings()),
        is_present: false,
    };
    let _runner_handle = tokio::spawn(async move {
        loop {
            runner.step().await;
        }
    });

    let outcome4 = try_poll_cycle(&worker, &bus, &mut tick, &cfg).await;
    assert!(matches!(outcome4, Err(PollError::Unplugged(_))));
    bus.write_disconnected(DisconnectReason::Unplugged);
    assert!(matches!(
        *bus.state.read(),
        DeviceState::Disconnected { .. }
    ));
    let _ = change_rx.changed().await;

    // 5. Asleep with no prior snapshot
    let state = Arc::new(RwLock::new(DeviceState::Disconnected {
        since: Instant::now(),
        reason: DisconnectReason::NeverConnected,
    }));
    let (change_tx, _change_rx) = tokio::sync::watch::channel(0);
    let bus = StateBus {
        state: state.clone(),
        change_tx,
    };

    let (tx, rx) = mpsc::channel(32);
    let worker = DeviceWorker::spawn_mock(tx);
    let mut runner = MockDeviceRunner {
        rx,
        power_res: Err(TransportError::Timeout(Duration::from_secs(1))),
        settings_res: Ok(default_settings()),
        is_present: true,
    };
    let _runner_handle = tokio::spawn(async move {
        loop {
            runner.step().await;
        }
    });

    let outcome5 = try_poll_cycle(&worker, &bus, &mut tick, &cfg).await;
    assert!(matches!(outcome5, Err(PollError::Unplugged(ref s)) if s == "NeverConnected"));

    Ok(())
}
