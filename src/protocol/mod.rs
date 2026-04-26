use crate::state::*;
use std::collections::BTreeMap;

pub const REPORT_ID: u8 = 0x08;

/// HID command bytes (`payload[1]`).
pub mod cmd {
    pub const POWER: u8 = 0x04;
    pub const SETTINGS: u8 = 0x08;
}

/// Settings memory layout. Each settings-read response carries 10 bytes;
/// chunks tile the address range [SETTINGS_MIN_ADDR, SETTINGS_MAX_ADDR].
pub const SETTINGS_CHUNK_LEN: u8 = 10;
pub const SETTINGS_MIN_ADDR: u8 = 0x00;
pub const SETTINGS_MAX_ADDR: u8 = 0xb8;

/// Byte addresses inside the device's settings memory.
/// Verified against `pulsar_status.py:173-192`.
pub mod settings_addr {
    pub const POLLING_RATE: u8 = 0x00;
    pub const DPI_SLOT_COUNT: u8 = 0x02;
    pub const DPI_SLOT: u8 = 0x04;
    pub const LIFT_OFF: u8 = 0x0a;
    pub const LED_EFFECT: u8 = 0x4c;
    pub const LED_ENABLED: u8 = 0x52;
    pub const DEBOUNCE_MS: u8 = 0xa9;
    pub const MOTION_SYNC: u8 = 0xab;
    pub const ANGLE_SNAPPING: u8 = 0xaf;
    pub const LOD_RIPPLE: u8 = 0xb1;
    pub const AUTO_SLEEP: u8 = 0xb7;
}

/// Raw byte codes the device uses to encode polling rate at `settings_addr::POLLING_RATE`.
///
/// Codes are one-hot bitmasks. Bits 0..=3 are the documented "standard" rates
/// (`pulsar_status.py:110`): each leftward shift halves the rate from 1000 Hz.
/// Bits 4..=6 (`HZ_2000` / `HZ_4000` / `HZ_8000`) are inferred from the same
/// pattern doubling outward — not documented by the Python reference, but the
/// dongle is named "8K", and a real X2 8K reports `0x40` when configured for
/// its top rate. Treat the high-rate codes as best-effort until cross-checked
/// by toggling 4000 Hz on the mouse and observing the byte.
pub mod polling_code {
    pub const HZ_1000: u8 = 0x01;
    pub const HZ_500: u8 = 0x02;
    pub const HZ_250: u8 = 0x04;
    pub const HZ_125: u8 = 0x08;
    pub const HZ_2000: u8 = 0x10;
    pub const HZ_4000: u8 = 0x20;
    pub const HZ_8000: u8 = 0x40;
}

/// Raw byte codes for `settings_addr::LIFT_OFF`.
pub mod lift_off_code {
    pub const MM1: u8 = 0x01;
    pub const MM2: u8 = 0x02;
}

/// Raw byte codes for `settings_addr::LED_EFFECT`. LED is "off" when
/// `settings_addr::LED_ENABLED` is zero, regardless of the effect byte.
pub mod led_effect_code {
    pub const STEADY: u8 = 0x01;
    pub const BREATHE: u8 = 0x02;
}

/// Raw byte codes for the charge-state byte in the power response (`frame[7]`).
pub mod charge_code {
    pub const DISCHARGING: u8 = 0x00;
    pub const CHARGING: u8 = 0x01;
}

/// Multiplier for the auto-sleep raw byte (raw * 10 = seconds).
pub const AUTO_SLEEP_SECONDS_PER_UNIT: u32 = 10;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("invalid frame length: expected 17, got {0}")]
    InvalidLength(usize),
    #[error("wrong report ID: expected 0x08, got 0x{0:02x}")]
    WrongReportId(u8),
    #[error("wrong command ID: expected 0x{expected:02x}, got 0x{got:02x}")]
    WrongCommand { expected: u8, got: u8 },
    #[error("checksum mismatch: expected 0x{expected:02x}, got 0x{got:02x}")]
    ChecksumMismatch { expected: u8, got: u8 },
}

pub fn checksum(bytes: &[u8; 16]) -> u8 {
    0x55u8.wrapping_sub(bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b)))
}

pub fn build_payload(cmd: u8, idx04: u8, idx05: u8, idx06: u8) -> [u8; 17] {
    let mut payload = [0u8; 17];
    payload[0] = REPORT_ID;
    payload[1] = cmd;
    payload[4] = idx04;
    payload[5] = idx05;
    payload[6] = idx06;

    let mut check_bytes = [0u8; 16];
    check_bytes.copy_from_slice(&payload[0..16]);
    payload[16] = checksum(&check_bytes);

    payload
}

pub fn verify_frame(frame: &[u8; 17], expected_cmd: u8) -> Result<(), ParseError> {
    if frame[0] != REPORT_ID {
        return Err(ParseError::WrongReportId(frame[0]));
    }
    if frame[1] != expected_cmd {
        return Err(ParseError::WrongCommand {
            expected: expected_cmd,
            got: frame[1],
        });
    }
    let mut check_bytes = [0u8; 16];
    check_bytes.copy_from_slice(&frame[0..16]);
    let calc = checksum(&check_bytes);
    if calc != frame[16] {
        return Err(ParseError::ChecksumMismatch {
            expected: calc,
            got: frame[16],
        });
    }
    Ok(())
}

pub fn parse_power(frame: &[u8; 17]) -> Result<Power, ParseError> {
    verify_frame(frame, cmd::POWER)?;

    let percent = BatteryPercent::new(frame[6]);
    let charge = match frame[7] {
        charge_code::DISCHARGING => ChargeState::Discharging,
        charge_code::CHARGING => ChargeState::Charging,
        other => ChargeState::Other(other),
    };
    let voltage_mv = u16::from_be_bytes([frame[8], frame[9]]);

    Ok(Power {
        percent,
        charge,
        voltage_mv,
    })
}

pub fn parse_settings_chunk(frame: &[u8; 17]) -> Result<(u8, [u8; 10]), ParseError> {
    verify_frame(frame, cmd::SETTINGS)?;
    let start_addr = frame[4];
    let mut chunk = [0u8; SETTINGS_CHUNK_LEN as usize];
    chunk.copy_from_slice(&frame[6..16]);
    Ok((start_addr, chunk))
}

pub fn interpret_settings(map: &BTreeMap<u8, u8>) -> Settings {
    let get_byte = |addr: u8| -> u8 {
        match map.get(&addr) {
            Some(v) => *v,
            None => unreachable!(
                "address 0x{:02x} not in map; caller must read all chunks",
                addr
            ),
        }
    };

    let polling = match get_byte(settings_addr::POLLING_RATE) {
        polling_code::HZ_1000 => PollingRate::Hz1000,
        polling_code::HZ_500 => PollingRate::Hz500,
        polling_code::HZ_250 => PollingRate::Hz250,
        polling_code::HZ_125 => PollingRate::Hz125,
        polling_code::HZ_2000 => PollingRate::Hz2000,
        polling_code::HZ_4000 => PollingRate::Hz4000,
        polling_code::HZ_8000 => PollingRate::Hz8000,
        other => PollingRate::Unknown(other),
    };

    let dpi_slot = get_byte(settings_addr::DPI_SLOT);
    let dpi_slot_count = get_byte(settings_addr::DPI_SLOT_COUNT);

    let lift_off = match get_byte(settings_addr::LIFT_OFF) {
        lift_off_code::MM1 => LiftOffDistance::Mm1,
        lift_off_code::MM2 => LiftOffDistance::Mm2,
        other => LiftOffDistance::Other(other),
    };

    let debounce_ms = get_byte(settings_addr::DEBOUNCE_MS);
    let auto_sleep_seconds =
        (get_byte(settings_addr::AUTO_SLEEP) as u32) * AUTO_SLEEP_SECONDS_PER_UNIT;

    let motion_sync = get_byte(settings_addr::MOTION_SYNC) != 0;
    let angle_snapping = get_byte(settings_addr::ANGLE_SNAPPING) != 0;
    let lod_ripple = get_byte(settings_addr::LOD_RIPPLE) != 0;

    let led = if get_byte(settings_addr::LED_ENABLED) == 0 {
        LedState::Off
    } else {
        match get_byte(settings_addr::LED_EFFECT) {
            led_effect_code::STEADY => LedState::Steady,
            led_effect_code::BREATHE => LedState::Breathe,
            other => LedState::Unknown(other),
        }
    };

    Settings {
        polling,
        dpi_slot,
        dpi_slot_count,
        lift_off,
        debounce_ms,
        auto_sleep_seconds,
        motion_sync,
        angle_snapping,
        lod_ripple,
        led,
    }
}
