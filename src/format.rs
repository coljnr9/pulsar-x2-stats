use crate::state::DeviceState;
use serde_json::json;

pub fn waybar(state: &DeviceState) -> serde_json::Value {
    match state {
        DeviceState::Connected { snapshot, .. } => {
            let charge_str = match snapshot.power.charge {
                crate::state::ChargeState::Discharging => "Discharging",
                crate::state::ChargeState::Charging => "Charging",
                crate::state::ChargeState::Other(v) => {
                    return json!({
                        "text": format!("{}%", snapshot.power.percent.get()),
                        "class": "warning",
                        "tooltip": format!("Battery {}%\nVoltage {}mV\nOther({})", snapshot.power.percent.get(), snapshot.power.voltage_mv, v),
                        "percentage": snapshot.power.percent.get()
                    });
                }
            };

            let class = if snapshot.power.charge == crate::state::ChargeState::Charging {
                "charging"
            } else if snapshot.power.percent.get() <= 10 {
                "critical"
            } else if snapshot.power.percent.get() <= 20 {
                "warning"
            } else {
                "normal"
            };

            json!({
                "text": format!("{}%", snapshot.power.percent.get()),
                "class": class,
                "tooltip": format!("Battery {}%\nVoltage {}V\n{}", snapshot.power.percent.get(), snapshot.power.voltage_mv as f64 / 1000.0, charge_str),
                "percentage": snapshot.power.percent.get()
            })
        }
        DeviceState::Asleep {
            last_snapshot,
            last_known_at,
            ..
        } => {
            let elapsed_secs = last_known_at.elapsed().as_secs();
            let time_str = if elapsed_secs < 60 {
                format!("{}s", elapsed_secs)
            } else if elapsed_secs < 3600 {
                format!("{}m", elapsed_secs / 60)
            } else {
                format!("{}h", elapsed_secs / 3600)
            };

            json!({
                "text": format!("💤 {}%", last_snapshot.power.percent.get()),
                "class": "sleep",
                "tooltip": format!("Battery {}% (last seen {} ago)\nMouse asleep", last_snapshot.power.percent.get(), time_str),
                "percentage": last_snapshot.power.percent.get()
            })
        }
        DeviceState::Disconnected { .. } => {
            json!({
                "text": "Disconnected",
                "class": "critical"
            })
        }
    }
}
