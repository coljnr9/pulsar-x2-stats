use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Instant;

pub type SharedState = Arc<RwLock<DeviceState>>;

pub struct StateBus {
    pub state: SharedState,
    pub change_tx: tokio::sync::watch::Sender<u64>,
}

impl StateBus {
    pub fn write_disconnected(&self, reason: DisconnectReason) {
        let mut w = self.state.write();
        let changed = !matches!(&*w, DeviceState::Disconnected {
                reason: current_reason,
                ..
            } if current_reason == &reason);

        if changed {
            *w = DeviceState::Disconnected {
                since: Instant::now(),
                reason,
            };
            self.notify();
        }
    }

    pub fn notify(&self) {
        let val = *self.change_tx.borrow();
        let _ = self.change_tx.send(val.wrapping_add(1));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum DeviceState {
    Disconnected {
        #[serde(skip)]
        since: Instant,
        reason: DisconnectReason,
    },
    Asleep {
        last_snapshot: Snapshot,
        #[serde(skip)]
        sleeping_since: Instant,
        #[serde(skip)]
        last_known_at: Instant,
    },
    Connected {
        snapshot: Snapshot,
        #[serde(skip)]
        last_polled: Instant,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DisconnectReason {
    NeverConnected,
    Unplugged,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Snapshot {
    pub power: Power,
    pub settings: Settings,
    pub profile: Option<u8>,
    #[serde(skip)]
    pub settings_last_read: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Power {
    pub percent: BatteryPercent,
    pub charge: ChargeState,
    pub voltage_mv: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct BatteryPercent(u8);

impl BatteryPercent {
    pub fn new(val: u8) -> Self {
        Self(val.min(100))
    }
    pub fn get(&self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChargeState {
    Discharging,
    Charging,
    Other(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Settings {
    pub polling: PollingRate,
    pub dpi_slot: u8,
    pub dpi_slot_count: u8,
    pub lift_off: LiftOffDistance,
    pub debounce_ms: u8,
    pub auto_sleep_seconds: u32,
    pub motion_sync: bool,
    pub angle_snapping: bool,
    pub lod_ripple: bool,
    pub led: LedState,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum PollingRate {
    Hz8000,
    Hz4000,
    Hz2000,
    Hz1000,
    Hz500,
    Hz250,
    Hz125,
    Unknown(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum LiftOffDistance {
    Mm1,
    Mm2,
    Other(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum LedState {
    Off,
    Steady,
    Breathe,
    Unknown(u8),
}
