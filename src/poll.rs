use crate::device::DeviceWorker;
use crate::state::{DeviceState, DisconnectReason, Snapshot, StateBus};
use crate::transport::TransportError;
use std::time::Duration;
use std::time::Instant;
use tokio::time::sleep;

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("poll error: {0}")]
    Poll(String),
}

pub struct PollConfig {
    pub poll_interval: Duration,
    pub asleep_poll_interval: Duration,
    pub settings_refresh_every: u64,
    pub backoff_max_secs: u64,
}

struct Backoff {
    current: Duration,
    max: Duration,
}

impl Backoff {
    fn new(initial: Duration, max: Duration) -> Self {
        Self {
            current: initial,
            max,
        }
    }
    fn next(&mut self) -> Duration {
        let ret = self.current;
        self.current = std::cmp::min(self.current * 2, self.max);
        ret
    }
    fn reset(&mut self) {
        self.current = Duration::from_secs(1);
    }
}

#[derive(Debug)]
pub enum PollOutcome {
    Connected,
    Asleep,
}

#[derive(Debug)]
pub enum PollError {
    Unplugged(String),
    Other(String),
}

impl From<PollError> for DisconnectReason {
    fn from(err: PollError) -> Self {
        match err {
            PollError::Unplugged(_) => DisconnectReason::Unplugged,
            PollError::Other(e) => DisconnectReason::Error(e),
        }
    }
}

pub async fn try_poll_cycle(
    worker: &DeviceWorker,
    bus: &StateBus,
    tick: &mut u64,
    cfg: &PollConfig,
) -> Result<PollOutcome, PollError> {
    let current_state = bus.state.read().clone();

    let power_res = worker.get_power().await;

    match power_res {
        Ok(power) => {
            let needs_settings = match &current_state {
                DeviceState::Connected { snapshot, .. } => {
                    (*tick).is_multiple_of(cfg.settings_refresh_every)
                        || snapshot.settings_last_read.elapsed()
                            > Duration::from_secs(
                                cfg.settings_refresh_every * cfg.poll_interval.as_secs() * 2,
                            )
                }
                DeviceState::Asleep { .. } => true,
                DeviceState::Disconnected { .. } => true,
            };

            let (settings, profile, settings_last_read) = if needs_settings {
                match worker.read_settings().await {
                    Ok(s) => {
                        let p = worker.get_active_profile().await.ok();
                        (s, p, Instant::now())
                    }
                    Err(e) => match &current_state {
                        DeviceState::Connected { snapshot, .. }
                        | DeviceState::Asleep {
                            last_snapshot: snapshot,
                            ..
                        } => (
                            snapshot.settings.clone(),
                            snapshot.profile,
                            snapshot.settings_last_read,
                        ),
                        _ => {
                            return Err(PollError::Other(format!(
                                "failed to read initial settings: {}",
                                e
                            )));
                        }
                    },
                }
            } else {
                match &current_state {
                    DeviceState::Connected { snapshot, .. }
                    | DeviceState::Asleep {
                        last_snapshot: snapshot,
                        ..
                    } => (
                        snapshot.settings.clone(),
                        snapshot.profile,
                        snapshot.settings_last_read,
                    ),
                    _ => unreachable!(),
                }
            };

            let new_snapshot = Snapshot {
                power: power.clone(),
                settings,
                profile,
                settings_last_read,
            };

            let new_state = DeviceState::Connected {
                snapshot: new_snapshot,
                last_polled: Instant::now(),
            };

            let mut w = bus.state.write();
            let changed = *w != new_state;
            if changed {
                tracing::info!(
                    "State changed to Connected: {}% / {} mV / {:?}",
                    power.percent.get(),
                    power.voltage_mv,
                    power.charge
                );
            } else {
                tracing::debug!("Polled: {}% / {} mV", power.percent.get(), power.voltage_mv);
            }
            *w = new_state;
            if changed {
                bus.notify();
            }

            *tick += 1;
            Ok(PollOutcome::Connected)
        }
        Err(TransportError::Timeout(_)) => {
            if let Ok(true) = worker.is_present().await {
                let new_state = match &current_state {
                    DeviceState::Connected { snapshot, .. } => DeviceState::Asleep {
                        last_snapshot: snapshot.clone(),
                        sleeping_since: Instant::now(),
                        last_known_at: Instant::now(),
                    },
                    DeviceState::Asleep {
                        last_snapshot,
                        sleeping_since,
                        ..
                    } => DeviceState::Asleep {
                        last_snapshot: last_snapshot.clone(),
                        sleeping_since: *sleeping_since,
                        last_known_at: Instant::now(),
                    },
                    DeviceState::Disconnected { .. } => {
                        return Err(PollError::Unplugged("NeverConnected".to_string()));
                    }
                };

                let mut w = bus.state.write();
                let changed = *w != new_state;
                if changed {
                    tracing::info!(
                        "State changed to Asleep (device still enumerated, no response)"
                    );
                } else {
                    tracing::debug!("Polled: still Asleep");
                }
                *w = new_state;
                if changed {
                    bus.notify();
                }

                Ok(PollOutcome::Asleep)
            } else {
                Err(PollError::Unplugged("Device not present".to_string()))
            }
        }
        Err(e) => {
            if let Ok(true) = worker.is_present().await {
                Err(PollError::Other(e.to_string()))
            } else {
                Err(PollError::Unplugged(e.to_string()))
            }
        }
    }
}

pub async fn run(worker: DeviceWorker, bus: StateBus, cfg: PollConfig) -> Result<(), DaemonError> {
    let mut backoff = Backoff::new(
        Duration::from_secs(1),
        Duration::from_secs(cfg.backoff_max_secs),
    );
    let mut tick: u64 = 0;
    loop {
        match try_poll_cycle(&worker, &bus, &mut tick, &cfg).await {
            Ok(PollOutcome::Connected) => {
                backoff.reset();
                sleep(cfg.poll_interval).await;
            }
            Ok(PollOutcome::Asleep) => {
                backoff.reset();
                sleep(cfg.asleep_poll_interval).await;
            }
            Err(PollError::Unplugged(e)) => {
                bus.write_disconnected(DisconnectReason::Unplugged);
                tracing::warn!("device unplugged; backing off (reason: {})", e);
                sleep(backoff.next()).await;
            }
            Err(PollError::Other(e)) => {
                bus.write_disconnected(DisconnectReason::Error(e.clone()));
                tracing::warn!("poll cycle failed; reconnecting: {}", e);
                sleep(backoff.next()).await;
            }
        }
    }
}
