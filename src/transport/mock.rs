use crate::transport::{MouseTransport, TransportError};
use std::collections::VecDeque;

pub struct MockTransport {
    pub responses: VecDeque<[u8; 17]>,
    pub recorded_writes: Vec<[u8; 17]>,
    pub present: bool,
}

impl MockTransport {
    pub fn new(responses: VecDeque<[u8; 17]>) -> Self {
        Self {
            responses,
            recorded_writes: Vec::new(),
            present: true,
        }
    }
}

impl MouseTransport for MockTransport {
    fn write_read(
        &mut self,
        payload: [u8; 17],
        expect_cmd: u8,
    ) -> Result<[u8; 17], TransportError> {
        self.recorded_writes.push(payload);
        if let Some(resp) = self.responses.pop_front() {
            if resp[1] == expect_cmd {
                Ok(resp)
            } else {
                Err(TransportError::UnexpectedCommand {
                    expected: expect_cmd,
                    got: resp[1],
                })
            }
        } else {
            Err(TransportError::Timeout(std::time::Duration::from_secs(1)))
        }
    }

    fn drain(&mut self) -> Result<(), TransportError> {
        Ok(())
    }

    fn is_present(&self) -> bool {
        self.present
    }
}
