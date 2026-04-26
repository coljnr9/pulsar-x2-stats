use crate::protocol;
use crate::state::{Power, Settings};
use crate::transport::hidapi_impl::HidApiTransport;
use crate::transport::{INTERFACE_NUMBER, MouseTransport, PRODUCT_ID, TransportError, VENDOR_ID};
use std::collections::BTreeMap;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

pub enum HidCommand {
    GetPower(oneshot::Sender<Result<Power, TransportError>>),
    ReadSettings(oneshot::Sender<Result<Settings, TransportError>>),
    GetActiveProfile(oneshot::Sender<Result<u8, TransportError>>),
    IsPresent(oneshot::Sender<bool>),
    Shutdown,
}

pub struct DeviceWorker {
    tx: mpsc::Sender<HidCommand>,
}

impl DeviceWorker {
    pub fn spawn() -> Self {
        let (tx, mut rx) = mpsc::channel(32);

        std::thread::spawn(move || {
            let mut transport_opt = HidApiTransport::open().ok();

            while let Some(cmd) = rx.blocking_recv() {
                match cmd {
                    HidCommand::Shutdown => break,
                    HidCommand::GetPower(reply) => {
                        let res = Self::handle_get_power(&mut transport_opt);
                        let _ = reply.send(res);
                    }
                    HidCommand::ReadSettings(reply) => {
                        let res = Self::handle_read_settings(&mut transport_opt);
                        let _ = reply.send(res);
                    }
                    HidCommand::GetActiveProfile(reply) => {
                        let res = Self::handle_get_profile(&mut transport_opt);
                        let _ = reply.send(res);
                    }
                    HidCommand::IsPresent(reply) => {
                        let res = Self::handle_is_present(&mut transport_opt);
                        let _ = reply.send(res);
                    }
                }
            }
        });

        Self { tx }
    }

    pub fn spawn_mock(tx: mpsc::Sender<HidCommand>) -> Self {
        Self { tx }
    }

    fn handle_is_present(transport_opt: &mut Option<HidApiTransport>) -> bool {
        if let Some(t) = transport_opt {
            t.is_present()
        } else {
            let api = match hidapi::HidApi::new() {
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

    fn ensure_transport(
        transport_opt: &mut Option<HidApiTransport>,
    ) -> Result<&mut HidApiTransport, TransportError> {
        if transport_opt.is_none() {
            match HidApiTransport::open() {
                Ok(t) => *transport_opt = Some(t),
                Err(e) => return Err(e),
            }
        }

        match transport_opt {
            Some(t) => Ok(t),
            None => unreachable!(),
        }
    }

    fn handle_get_power(
        transport_opt: &mut Option<HidApiTransport>,
    ) -> Result<Power, TransportError> {
        let transport = Self::ensure_transport(transport_opt)?;
        let payload = protocol::build_payload(protocol::cmd::POWER, 0, 0, 0);
        let resp = match transport.write_read(payload, protocol::cmd::POWER) {
            Ok(r) => r,
            Err(e) => {
                if !matches!(e, TransportError::Timeout(_)) {
                    *transport_opt = None;
                }
                return Err(e);
            }
        };

        protocol::parse_power(&resp).map_err(Into::into)
    }

    fn handle_read_settings(
        transport_opt: &mut Option<HidApiTransport>,
    ) -> Result<Settings, TransportError> {
        let transport = Self::ensure_transport(transport_opt)?;
        let mut map = BTreeMap::new();

        let chunk_count = protocol::SETTINGS_MAX_ADDR / protocol::SETTINGS_CHUNK_LEN + 1;
        for chunk_idx in 0u8..chunk_count {
            let start_addr = chunk_idx * protocol::SETTINGS_CHUNK_LEN;
            let payload = protocol::build_payload(
                protocol::cmd::SETTINGS,
                start_addr,
                protocol::SETTINGS_CHUNK_LEN,
                0,
            );

            let resp = match transport.write_read(payload, protocol::cmd::SETTINGS) {
                Ok(r) => r,
                Err(e) => {
                    if !matches!(e, TransportError::Timeout(_)) {
                        *transport_opt = None;
                    }
                    return Err(e);
                }
            };

            let (addr, chunk) = protocol::parse_settings_chunk(&resp)?;

            for (i, &byte) in chunk.iter().enumerate() {
                map.insert(addr + (i as u8), byte);
            }
        }

        Ok(protocol::interpret_settings(&map))
    }

    fn handle_get_profile(
        _transport_opt: &mut Option<HidApiTransport>,
    ) -> Result<u8, TransportError> {
        Ok(0)
    }

    pub async fn get_power(&self) -> Result<Power, TransportError> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(HidCommand::GetPower(tx)).await.map_err(|_| {
            TransportError::Io(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "worker dead").into(),
            )
        })?;
        rx.await.map_err(|_| {
            TransportError::Io(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "worker dead").into(),
            )
        })?
    }

    pub async fn read_settings(&self) -> Result<Settings, TransportError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(HidCommand::ReadSettings(tx))
            .await
            .map_err(|_| {
                TransportError::Io(
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "worker dead").into(),
                )
            })?;
        rx.await.map_err(|_| {
            TransportError::Io(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "worker dead").into(),
            )
        })?
    }

    pub async fn get_active_profile(&self) -> Result<u8, TransportError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(HidCommand::GetActiveProfile(tx))
            .await
            .map_err(|_| {
                TransportError::Io(
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "worker dead").into(),
                )
            })?;
        rx.await.map_err(|_| {
            TransportError::Io(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "worker dead").into(),
            )
        })?
    }

    pub async fn is_present(&self) -> Result<bool, TransportError> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(HidCommand::IsPresent(tx)).await.map_err(|_| {
            TransportError::Io(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "worker dead").into(),
            )
        })?;
        rx.await.map_err(|_| {
            TransportError::Io(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "worker dead").into(),
            )
        })
    }
}
