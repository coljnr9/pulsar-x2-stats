use pulsar_daemon::protocol::*;
use pulsar_daemon::state::*;
use std::collections::BTreeMap;

#[test]
fn test_checksum_vector() {
    let mut payload = [0u8; 17];
    payload[0] = 0x08;
    payload[1] = 0x04;

    let mut check_bytes = [0u8; 16];
    check_bytes.copy_from_slice(&payload[0..16]);
    let calc = checksum(&check_bytes);

    assert_eq!(calc, 0x49);
}

#[test]
fn test_build_payload_power() {
    let payload = build_payload(cmd::POWER, 0, 0, 0);
    assert_eq!(payload[0], 0x08);
    assert_eq!(payload[1], cmd::POWER);
    assert_eq!(payload[16], 0x49);
}

#[test]
fn test_parse_power_valid() -> Result<(), ParseError> {
    // Construct a valid power frame manually
    let mut frame = [0u8; 17];
    frame[0] = 0x08;
    frame[1] = 0x04;
    frame[6] = 73; // 73% battery
    frame[7] = 0; // Discharging
    frame[8] = 0x0F; // Voltage MSB
    frame[9] = 0xAC; // Voltage LSB (4012 mV)

    let mut check_bytes = [0u8; 16];
    check_bytes.copy_from_slice(&frame[0..16]);
    frame[16] = checksum(&check_bytes);

    let power = parse_power(&frame)?;
    assert_eq!(power.percent.get(), 73);
    assert_eq!(power.charge, ChargeState::Discharging);
    assert_eq!(power.voltage_mv, 4012);
    Ok(())
}

#[test]
fn test_parse_power_invalid_id() {
    let mut frame = [0u8; 17];
    frame[0] = 0x09; // Wrong report ID
    frame[1] = 0x04;
    let mut check_bytes = [0u8; 16];
    check_bytes.copy_from_slice(&frame[0..16]);
    frame[16] = checksum(&check_bytes);

    let result = parse_power(&frame);
    assert!(matches!(result, Err(ParseError::WrongReportId(0x09))));
}

#[test]
fn test_parse_power_invalid_cmd() {
    let mut frame = [0u8; 17];
    frame[0] = 0x08;
    frame[1] = 0x05; // Wrong command
    let mut check_bytes = [0u8; 16];
    check_bytes.copy_from_slice(&frame[0..16]);
    frame[16] = checksum(&check_bytes);

    let result = parse_power(&frame);
    assert!(matches!(
        result,
        Err(ParseError::WrongCommand {
            expected: 0x04,
            got: 0x05
        })
    ));
}

#[test]
fn test_parse_power_invalid_checksum() {
    let mut frame = [0u8; 17];
    frame[0] = 0x08;
    frame[1] = 0x04;
    let mut check_bytes = [0u8; 16];
    check_bytes.copy_from_slice(&frame[0..16]);
    frame[16] = checksum(&check_bytes).wrapping_add(1); // Invalid checksum

    let result = parse_power(&frame);
    assert!(matches!(result, Err(ParseError::ChecksumMismatch { .. })));
}

#[test]
fn test_interpret_settings() {
    use settings_addr::*;

    let mut map = BTreeMap::new();
    map.insert(POLLING_RATE, polling_code::HZ_1000); // 1000 Hz
    map.insert(DPI_SLOT, 1); // DPI slot
    map.insert(DPI_SLOT_COUNT, 4); // DPI slot count
    map.insert(LIFT_OFF, lift_off_code::MM2); // 2mm lift off
    map.insert(DEBOUNCE_MS, 4); // 4ms debounce
    map.insert(AUTO_SLEEP, 6); // 60s auto sleep
    map.insert(MOTION_SYNC, 0x01); // Motion sync on
    map.insert(ANGLE_SNAPPING, 0x00); // Angle snapping off
    map.insert(LOD_RIPPLE, 0x01); // LOD ripple on
    map.insert(LED_EFFECT, led_effect_code::STEADY); // Steady LED effect
    map.insert(LED_ENABLED, 0x01); // LED enabled

    let settings = interpret_settings(&map);
    assert_eq!(settings.polling, PollingRate::Hz1000);
    assert_eq!(settings.dpi_slot, 1);
    assert_eq!(settings.dpi_slot_count, 4);
    assert_eq!(settings.lift_off, LiftOffDistance::Mm2);
    assert_eq!(settings.debounce_ms, 4);
    assert_eq!(settings.auto_sleep_seconds, 60);
    assert!(settings.motion_sync);
    assert!(!settings.angle_snapping);
    assert!(settings.lod_ripple);
    assert_eq!(settings.led, LedState::Steady);
}

#[test]
fn test_interpret_settings_unknowns() {
    use settings_addr::*;

    let mut map = BTreeMap::new();
    map.insert(POLLING_RATE, 0xFF); // Unknown polling
    map.insert(DPI_SLOT, 0); // dpi_slot
    map.insert(DPI_SLOT_COUNT, 0); // dpi_slot_count
    map.insert(LIFT_OFF, 0xFF); // Unknown LOD
    map.insert(DEBOUNCE_MS, 0); // debounce_ms
    map.insert(AUTO_SLEEP, 0); // auto_sleep_seconds
    map.insert(MOTION_SYNC, 0); // motion_sync false
    map.insert(ANGLE_SNAPPING, 0); // angle_snapping false
    map.insert(LOD_RIPPLE, 0); // lod_ripple false
    map.insert(LED_EFFECT, 0xFF); // Unknown LED effect
    map.insert(LED_ENABLED, 0x01); // LED enabled (so we test the effect fallthrough)

    let settings = interpret_settings(&map);
    assert_eq!(settings.polling, PollingRate::Unknown(0xFF));
    assert_eq!(settings.lift_off, LiftOffDistance::Other(0xFF));
    assert_eq!(settings.led, LedState::Unknown(0xFF));
}
