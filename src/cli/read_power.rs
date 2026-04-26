use crate::protocol;
use crate::transport;
use crate::transport::hidapi_impl::HidApiTransport;
use crate::transport::{MouseTransport, TransportError};
use std::process::exit;

pub fn run() {
    let mut transport = match HidApiTransport::open() {
        Ok(t) => t,
        Err(e) => {
            if let TransportError::NotFound { .. } = e {
                eprintln!(
                    "no Pulsar dongle detected (vendor=0x{:04x} product=0x{:04x})",
                    transport::VENDOR_ID,
                    transport::PRODUCT_ID
                );
                exit(2);
            }
            eprintln!("{}", e);
            exit(1);
        }
    };

    let payload = protocol::build_payload(protocol::cmd::POWER, 0, 0, 0);

    match transport.write_read(payload, protocol::cmd::POWER) {
        Ok(resp) => match protocol::parse_power(&resp) {
            Ok(power) => {
                let charge_str = match power.charge {
                    crate::state::ChargeState::Discharging => "Discharging",
                    crate::state::ChargeState::Charging => "Charging",
                    crate::state::ChargeState::Other(v) => {
                        println!(
                            "{}% / {} mV / Other({})",
                            power.percent.get(),
                            power.voltage_mv,
                            v
                        );
                        exit(0);
                    }
                };
                println!(
                    "{}% / {} mV / {}",
                    power.percent.get(),
                    power.voltage_mv,
                    charge_str
                );
                exit(0);
            }
            Err(e) => {
                eprintln!("{}", e);
                exit(1);
            }
        },
        Err(TransportError::Timeout(_)) => {
            if transport.is_present() {
                eprintln!("device asleep (no response within 1s; mouse is probably idle)");
                exit(0);
            } else {
                eprintln!(
                    "no Pulsar dongle detected (vendor=0x{:04x} product=0x{:04x})",
                    transport::VENDOR_ID,
                    transport::PRODUCT_ID
                );
                exit(2);
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    }
}
