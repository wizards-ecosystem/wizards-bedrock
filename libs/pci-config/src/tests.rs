use super::*;
use crate::probe::probe_bars_unleased;

mod probe;

#[derive(Clone, Debug)]
struct FakeConfig {
    command: u16,
    status: u16,
    bars: [u32; BARS],
    trace: Vec<ProbeAccess>,
}

impl FakeConfig {
    fn device() -> Self {
        Self {
            command: command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER,
            status: 0xA55A,
            bars: [0x8000_0004, 1, 0x0000_C001, 0, 0, 0],
            trace: Vec::new(),
        }
    }

    fn access(&mut self, event: ProbeAccess) -> Result<u32, ()> {
        self.trace.push(event);
        Ok(match event {
            ProbeAccess::ReadCommand => u32::from(self.command) | (u32::from(self.status) << 16),
            ProbeAccess::WriteCommand(command) => {
                self.command = command;
                0
            }
            ProbeAccess::ReadBar(index) => self.bars[index],
            ProbeAccess::WriteBar(index, value) => {
                self.bars[index] = value;
                0
            }
        })
    }
}

#[test]
fn an_address_encodes_as_a_configuration_cycle() {
    let at = Address::new(0, 2, 0, header::VENDOR).expect("a real address");
    assert_eq!(at.bits(), 0x8000_1000);
}
