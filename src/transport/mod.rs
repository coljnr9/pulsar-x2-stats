use std::time::Duration;

pub mod hidapi_impl;
pub mod mock;

pub const VENDOR_ID: u16 = 0x3710;
pub const PRODUCT_ID: u16 = 0x5406;
pub const INTERFACE_NUMBER: i32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("device not found (vendor=0x{vendor:04x} product=0x{product:04x})")]
    NotFound { vendor: u16, product: u16 },
    #[error("HID I/O: {0}")]
    Io(#[from] hidapi::HidError),
    #[error("response timeout after {0:?}")]
    Timeout(Duration),
    #[error("checksum mismatch: expected 0x{expected:02x}, got 0x{got:02x}")]
    Checksum { expected: u8, got: u8 },
    #[error("unexpected response command: expected 0x{expected:02x}, got 0x{got:02x}")]
    UnexpectedCommand { expected: u8, got: u8 },
    #[error("protocol error: {0}")]
    Protocol(#[from] crate::protocol::ParseError),
}

pub trait MouseTransport: Send {
    fn write_read(&mut self, payload: [u8; 17], expect_cmd: u8)
    -> Result<[u8; 17], TransportError>;
    fn drain(&mut self) -> Result<(), TransportError>;
    fn is_present(&self) -> bool;
}
