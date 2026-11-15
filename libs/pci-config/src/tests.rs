use super::*;
use crate::probe::{probe_bars_unleased, restore_staged_probe};

mod bar;
mod probe;

const TRACE_CAPACITY: usize = 96;

#[derive(Clone, Debug)]
struct FakeConfig {
    command: u16,
    status: u16,
    bars: [u32; BARS],
    masks: [u32; BARS],
    trace: [Option<ProbeAccess>; TRACE_CAPACITY],
    calls: usize,
    fail_at: Option<usize>,
    ignore_write_at: Option<usize>,
    bar_writes_with_bus_master: usize,
}

impl FakeConfig {
    fn device() -> Self {
        Self {
            command: command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER,
            status: 0xA55A,
            bars: [0x8000_0004, 1, 0x0000_C001, 0, 0, 0],
            masks: [0xFFFF_C004, u32::MAX, 0xFFFF_FFE1, 0, 0, 0],
            trace: [None; TRACE_CAPACITY],
            calls: 0,
            fail_at: None,
            ignore_write_at: None,
            bar_writes_with_bus_master: 0,
        }
    }

    fn access(&mut self, event: ProbeAccess) -> Result<u32, usize> {
        let call = self.calls;
        self.trace[call] = Some(event);
        self.calls += 1;

        let answer = match event {
            ProbeAccess::ReadCommand => u32::from(self.command) | (u32::from(self.status) << 16),
            ProbeAccess::WriteCommand(command) => {
                if self.ignore_write_at != Some(call) {
                    self.command = command;
                }
                0
            }
            ProbeAccess::ReadBar(index) => {
                if self.bars[index] == u32::MAX {
                    self.masks[index]
                } else {
                    self.bars[index]
                }
            }
            ProbeAccess::WriteBar(index, value) => {
                if self.command & command::BUS_MASTER != 0 {
                    self.bar_writes_with_bus_master += 1;
                }
                if self.ignore_write_at != Some(call) {
                    self.bars[index] = value;
                }
                0
            }
        };
        if self.fail_at == Some(call) {
            Err(call)
        } else {
            Ok(answer)
        }
    }

    fn events(&self) -> &[Option<ProbeAccess>] {
        &self.trace[..self.calls]
    }
}

fn probe_bars<E>(
    registers: usize,
    mut access: impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<ProbedBars, ProbeError<E>> {
    let staged = probe_bars_unleased(registers, &mut access).map_err(|failure| failure.reason)?;
    restore_staged_probe(staged, &mut access).map_err(|failure| failure.reason)
}

#[test]
fn an_address_that_cannot_be_encoded_is_refused() {
    assert!(Address::new(0, DEVICES, 0, 0).is_none(), "device 32");
    assert!(Address::new(0, 0, FUNCTIONS, 0).is_none(), "function 8");
    assert!(
        Address::new(0, 0, 0, HEADER_BYTES).is_none(),
        "past the header"
    );
    for misaligned in [1u16, 2, 3, 0x0D, 0xFF] {
        assert!(
            Address::new(0, 0, 0, misaligned).is_none(),
            "offset {misaligned:#x} is not a dword boundary"
        );
    }

    let at = Address::new(0, 1, 0, 0).expect("valid");
    assert!(at.at(0x0D).is_none());
    assert!(at.at(HEADER_BYTES).is_none());
    assert_eq!(at.at(0x10).expect("valid").bits() as u16 & 0xFF, 0x10);
    assert_eq!(at.at(0x10).expect("valid").device(), 1, "the rest is kept");
}

#[test]
fn a_fence_that_cannot_verify_its_restore_never_revives_bus_master() {
    let mut functions = FunctionClaims::<1>::new();
    let at = Address::new(0, 1, 0, 0).expect("valid");
    let authority = functions
        .claim(at, HeaderType::from_bits(0))
        .expect("claimed");
    let original = command::MEMORY_SPACE | command::BUS_MASTER;
    let mut reads = 0;
    let failure = fence_bus_master(&mut functions, &authority, |_, access| match access {
        CommandAccess::Read => {
            reads += 1;
            Some(u32::from(if reads == 1 { original } else { 0x0100 }))
        }
        CommandAccess::Write(_) => Some(0),
    })
    .expect_err("the restore cannot be verified");
    assert_eq!(failure.reason(), EnableError::Restore);

    let state = failure.into_state().expect("restore authority");
    let mut revived = false;
    let settled = original & !command::BUS_MASTER;
    let restored = restore_command(
        &mut functions,
        &authority,
        state,
        |_, access| match access {
            CommandAccess::Read => Some(u32::from(settled)),
            CommandAccess::Write(word) => {
                revived |= word & command::BUS_MASTER != 0;
                Some(0)
            }
        },
    );
    assert!(restored.is_ok());
    assert!(!revived);
}
