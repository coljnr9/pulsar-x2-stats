use crate::protocol;
use crate::transport::{INTERFACE_NUMBER, MouseTransport, PRODUCT_ID, TransportError, VENDOR_ID};
use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

pub struct HidApiTransport {
    device: HidDevice,
}

impl HidApiTransport {
    pub fn open() -> Result<Self, TransportError> {
        let api = HidApi::new()?;

        let mut target_path = None;
        for device_info in api.device_list() {
            if device_info.vendor_id() == VENDOR_ID
                && device_info.product_id() == PRODUCT_ID
                && device_info.interface_number() == INTERFACE_NUMBER
            {
                target_path = Some(device_info.path().to_owned());
                break;
            }
        }

        let path = match target_path {
            Some(p) => p,
            None => {
                return Err(TransportError::NotFound {
                    vendor: VENDOR_ID,
                    product: PRODUCT_ID,
                });
            }
        };

        let device = api.open_path(&path)?;
        device.set_blocking_mode(true)?;

        Ok(Self { device })
    }
}

impl MouseTransport for HidApiTransport {
    fn write_read(
        &mut self,
        payload: [u8; 17],
        expect_cmd: u8,
    ) -> Result<[u8; 17], TransportError> {
        self.drain()?;

        // 17-byte write form
        self.device.write(&payload)?;

        let deadline = Instant::now() + Duration::from_secs(1);
        let mut buf = [0u8; 17];

        loop {
            if Instant::now() > deadline {
                return Err(TransportError::Timeout(Duration::from_secs(1)));
            }

            // Read with 100ms timeout to avoid hard blocking
            let read_bytes = self.device.read_timeout(&mut buf, 100)?;

            if read_bytes == 17 {
                // Verify the report ID and command
                if buf[0] == protocol::REPORT_ID && buf[1] == expect_cmd {
                    let mut check_bytes = [0u8; 16];
                    check_bytes.copy_from_slice(&buf[0..16]);
                    let calc = protocol::checksum(&check_bytes);
                    if calc != buf[16] {
                        return Err(TransportError::Checksum {
                            expected: calc,
                            got: buf[16],
                        });
                    }
                    return Ok(buf);
                } else if buf[0] == protocol::REPORT_ID && buf[1] != expect_cmd {
                    // It's a valid frame but wrong command, keep reading?
                    // Let's log it maybe, but for now we continue
                }
            }
        }
    }

    fn drain(&mut self) -> Result<(), TransportError> {
        self.device.set_blocking_mode(false)?;
        let mut buf = [0u8; 32];
        loop {
            match self.device.read(&mut buf) {
                Ok(n) if n > 0 => continue,
                _ => break,
            }
        }
        self.device.set_blocking_mode(true)?;
        Ok(())
    }

    fn is_present(&self) -> bool {
        let api = match HidApi::new() {
            Ok(api) => api,
            Err(_) => return false,
        };

        for device_info in api.device_list() {
            if device_info.vendor_id() == VENDOR_ID
                && device_info.product_id() == PRODUCT_ID
                && device_info.interface_number() == INTERFACE_NUMBER
            {
                return true;
            }
        }
        false
    }
}
