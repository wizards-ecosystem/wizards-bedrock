use super::*;
use crate::claims::FunctionLease;
use crate::probe::{probe_bars_unleased, restore_staged_probe};

mod bar;
mod bridges;
mod claims;
mod command_state;
mod fence;
mod memory;
mod probe;
mod topology;

const TRACE_CAPACITY: usize = 96;
const COMMAND_TRACE_CAPACITY: usize = 16;

#[derive(Debug)]
struct FakeCommand {
    command: u16,
    status: u16,
    writable: u16,
    injected_read: Option<(usize, u16)>,
    trace: [Option<CommandAccess>; COMMAND_TRACE_CAPACITY],
    calls: usize,
    fail_at: Option<usize>,
}

impl FakeCommand {
    fn new(command: u16, writable: u16) -> Self {
        Self {
            command,
            status: 0xA55A,
            writable,
            injected_read: None,
            trace: [None; COMMAND_TRACE_CAPACITY],
            calls: 0,
            fail_at: None,
        }
    }

    fn access(&mut self, event: CommandAccess) -> Option<u32> {
        let call = self.calls;
        self.trace[call] = Some(event);
        self.calls += 1;
        if self.fail_at == Some(call) {
            return None;
        }

        match event {
            CommandAccess::Read => {
                let injected = self
                    .injected_read
                    .filter(|(at, _)| *at == call)
                    .map_or(0, |(_, bits)| bits);
                Some(u32::from(self.command | injected) | (u32::from(self.status) << 16))
            }
            CommandAccess::Write(value) => {
                self.command = (self.command & !self.writable) | (value & self.writable);
                Some(0)
            }
        }
    }

    fn events(&self) -> &[Option<CommandAccess>] {
        &self.trace[..self.calls]
    }
}

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

fn fake_command_access(fake: &mut FakeConfig, access: CommandAccess) -> Option<u32> {
    let event = match access {
        CommandAccess::Read => ProbeAccess::ReadCommand,
        CommandAccess::Write(value) => ProbeAccess::WriteCommand(value),
    };
    fake.access(event).ok()
}

fn command_address(function: u8) -> Address {
    Address::new(0, 1, function, header::BAR0).expect("a valid test function")
}

fn authority(at: Address, header: HeaderType) -> FunctionAuthority {
    FunctionAuthority {
        lease: FunctionLease {
            registry: 1,
            at: at.function_base(),
            generation: 1,
        },
        registers: header
            .bar_registers()
            .expect("a test authority needs a supported header"),
    }
}

fn device_authority(at: Address) -> FunctionAuthority {
    authority(at, HeaderType::from_bits(0))
}

fn state_authority(state: &CommandState) -> FunctionAuthority {
    FunctionAuthority {
        lease: state.lease.function,
        registers: BARS,
    }
}

fn enable_bus_master<const N: usize>(
    leases: &mut CommandLeases<N>,
    at: Address,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    crate::command_state::enable_bus_master_with_leases(leases, &device_authority(at), access)
}

fn enable_memory_space<const N: usize>(
    leases: &mut CommandLeases<N>,
    at: Address,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    crate::command_state::enable_memory_space_with_leases(leases, &device_authority(at), access)
}

fn admit_bus_master<const N: usize>(
    leases: &mut CommandLeases<N>,
    state: &mut CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    crate::command_state::admit_bus_master_with_leases(leases, state.lease.function, state, access)
}

fn disable_bus_master_for_retirement<const N: usize>(
    leases: &mut CommandLeases<N>,
    state: &mut CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    crate::command_state::disable_bus_master_for_retirement_with_leases(
        leases,
        state.lease.function,
        state,
        access,
    )
}

fn restore_command<const N: usize>(
    leases: &mut CommandLeases<N>,
    state: CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), RestoreFailure> {
    let authority = state_authority(&state);
    crate::command_state::restore_command_with_leases(leases, &authority, state, access)
}

fn commit_command<const N: usize>(
    leases: &mut CommandLeases<N>,
    state: CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), RestoreFailure> {
    let authority = state_authority(&state);
    crate::command_state::commit_command_with_leases(leases, &authority, state, access)
}

fn probe_bars<E>(
    registers: usize,
    mut access: impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<ProbedBars, ProbeError<E>> {
    let staged = probe_bars_unleased(registers, &mut access).map_err(|failure| failure.reason)?;
    restore_staged_probe(staged, &mut access).map_err(|failure| failure.reason)
}

fn duplicate_state(state: &CommandState) -> CommandState {
    CommandState {
        lease: state.lease,
        before: state.before,
        installed: state.installed,
        phase: state.phase,
    }
}

// Pairwise-distinct fields expose every accidental swap.
#[test]
fn an_address_puts_every_field_where_the_manual_says() {
    let at = Address::new(0x12, 0x1A, 0x05, 0x3C).expect("a valid address");

    assert_eq!(at.bits() >> 31, 1, "the enable bit");
    assert_eq!(at.bus(), 0x12);
    assert_eq!(at.device(), 0x1A);
    assert_eq!(at.function(), 0x05);
    assert_eq!(at.bits() as u16 & 0xFF, 0x3C);
    assert_eq!(at.bits(), 0x8012_D53C);
}

#[test]
fn every_address_round_trips_through_its_fields() {
    for bus in [0u8, 1, 0x7F, 0x80, 0xFE, 0xFF] {
        for device in 0..DEVICES {
            for function in 0..FUNCTIONS {
                for offset in (0..HEADER_BYTES).step_by(4) {
                    let at = Address::new(bus, device, function, offset).expect("all encodable");
                    assert_eq!(
                        (
                            at.bus(),
                            at.device(),
                            at.function(),
                            at.bits() as u16 & 0xFF,
                        ),
                        (bus, device, function, offset)
                    );
                }
            }
        }
    }
}

#[test]
fn every_offset_of_one_function_has_one_identity() {
    for start in (0..HEADER_BYTES).step_by(4) {
        let identity = Address::new(0x12, 0x1A, 5, start).expect("valid");
        for replacement in (0..HEADER_BYTES).step_by(4) {
            let alias = identity.at(replacement).expect("valid offset");
            assert_eq!(
                (
                    alias.bus(),
                    alias.device(),
                    alias.function(),
                    alias.bits() as u16 & 0xFF
                ),
                (0x12, 0x1A, 5, replacement)
            );
            assert_eq!(alias.function_base(), identity.function_base());
        }
    }
    let identity = Address::new(0x12, 0x1A, 5, 0).expect("valid");
    assert_ne!(identity.at(header::CLASS), identity.at(header::BAR0));
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
fn the_class_word_is_read_as_four_separate_bytes() {
    let class = Class::from_bits(0x0C_03_30_A1);
    assert_eq!(class.class, 0x0C);
    assert_eq!(class.subclass, 0x03);
    assert_eq!(class.interface, 0x30);
    assert_eq!(class.revision, 0xA1);
}

#[test]
fn nvme_is_all_three_bytes_and_its_neighbours_are_not() {
    assert!(Class::from_bits(0x01_08_02_00).is_nvme());

    for near in [
        0x01_08_01_00u32, // NVMHCI
        0x01_08_03_00,    // a prog-if this driver does not know
        0x01_06_02_00,    // SATA
        0x02_08_02_00,    // a network controller with the same tail
    ] {
        assert!(
            !Class::from_bits(near).is_nvme(),
            "{near:#010x} is not NVM Express"
        );
    }
}

#[test]
fn a_bridge_is_recognised_and_nothing_else_is() {
    assert!(Class::from_bits(0x06_04_00_00).is_bridge());
    assert!(
        !Class::from_bits(0x06_00_00_00).is_bridge(),
        "a host bridge"
    );
    assert!(
        !Class::from_bits(0x06_01_00_00).is_bridge(),
        "an ISA bridge"
    );
    assert!(!Class::from_bits(0x01_08_02_00).is_bridge());
}

#[test]
fn the_header_type_byte_is_a_layout_and_a_flag() {
    let plain = HeaderType::from_bits(0x00);
    assert!(!plain.is_multifunction());
    assert_eq!(plain.layout(), 0);
    assert!(!plain.is_bridge());

    let multi_bridge = HeaderType::from_bits(0x81);
    assert!(multi_bridge.is_multifunction());
    assert_eq!(multi_bridge.layout(), 1);
    assert!(multi_bridge.is_bridge());

    let multi_plain = HeaderType::from_bits(0x80);
    assert!(multi_plain.is_multifunction());
    assert_eq!(multi_plain.layout(), 0);
    assert!(!multi_plain.is_bridge());
}

#[test]
fn an_absent_function_reads_as_all_ones() {
    assert!(is_absent(0xFFFF));
    assert!(!is_absent(0x8086), "Intel");
    assert!(!is_absent(0x1B36), "QEMU");
    assert!(!is_absent(0x15AD), "VMware");
    assert!(!is_absent(0x0000), "a vendor of zero is odd, not absent");
}

#[test]
fn a_bridge_secondary_bus_is_read_from_its_own_byte() {
    let bits = 0x0044_3322u32 << 8 | 0x11;
    assert_eq!(secondary_bus(bits), 0x22);
}

#[test]
fn a_bridge_subordinate_bus_is_read_from_its_own_byte() {
    let bits = 0x4433_2211u32;
    assert_eq!(subordinate_bus(bits), 0x33);
}

#[test]
fn a_bridge_primary_bus_is_read_from_its_own_byte() {
    let bits = 0x4433_2211;
    assert_eq!(primary_bus(bits), 0x11);
}
