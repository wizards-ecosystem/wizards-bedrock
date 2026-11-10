use core::fmt;

use crate::bar::io_window_size;
use crate::claims::FunctionLease;
use crate::{
    Address, BARS, Bar, CommandLeases, CommandProbe, CommandState, EnableError, FunctionAuthority,
    FunctionClaims, MemoryWidth, PROBE_DISABLED, memory_window_size,
};

/// One configuration-space cycle in a BAR-probe transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProbeAccess {
    /// Only the low sixteen bits are consumed.
    ReadCommand,
    /// Write command bits with a zero status half.
    WriteCommand(u16),
    ReadBar(usize),
    WriteBar(usize, u32),
}

/// One decoded and sized BAR window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Window {
    pub bar: Bar,
    /// How many bytes or ports it covers.
    pub size: u64,
}

/// Every window discovered by one BAR-restored probe transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProbedBars {
    pub(crate) function: Option<FunctionLease>,
    pub(crate) windows: [Option<Window>; BARS],
}

/// A completed BAR probe whose safe command state remains exclusively owned.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "the staged command must be committed or restored before releasing its function"]
pub struct StagedProbe {
    bars: ProbedBars,
    command: CommandState,
}

impl StagedProbe {
    /// Separate the discovered windows from their command restoration owner.
    pub fn into_parts(self) -> (ProbedBars, CommandState) {
        (self.bars, self.command)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct BarSlots {
    low: usize,
    high: Option<usize>,
}

impl ProbedBars {
    pub fn window(&self, index: usize) -> Option<Window> {
        self.windows.get(index).copied().flatten()
    }

    /// All slots, including the empty high halves of wide BARs.
    pub const fn into_windows(self) -> [Option<Window>; BARS] {
        self.windows
    }
}

/// Why a BAR-probe transaction could not produce windows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ProbeError<E> {
    /// The function or command-transition reservation was unavailable.
    Unavailable(EnableError),
    /// The configuration mechanism refused an access before rollback failed.
    Access(E),
    /// The command register did not accept the exact decode-off word.
    Refused(u16),
    /// At least one original register could not be put back.
    Restore(E),
    /// A restoration readback did not equal the captured register word.
    RestoreChanged,
    /// The header layout, slot use or encoding is unsupported, or an I/O BAR leaves port space.
    Malformed,
    /// A device returned a mask or base that cannot describe its window.
    InvalidWindow,
}

impl<E: fmt::Display> fmt::Display for ProbeError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(reason) => {
                write!(f, "bar probe authority is unavailable: {reason}")
            }
            Self::Access(reason) => write!(f, "configuration access failed: {reason}"),
            Self::Refused(observed) => write!(
                f,
                "command register refused decode-off value and read back {observed:#06x}"
            ),
            Self::Restore(reason) => write!(f, "configuration restoration failed: {reason}"),
            Self::RestoreChanged => {
                f.write_str("configuration restoration changed the captured register state")
            }
            Self::Malformed => f.write_str("the function has an unsupported BAR layout"),
            Self::InvalidWindow => f.write_str("a BAR returned an invalid memory window"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for ProbeError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Unavailable(reason) => Some(reason),
            Self::Access(reason) | Self::Restore(reason) => Some(reason),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProbeRestorePhase {
    Bars,
    Command,
}

#[derive(PartialEq, Eq, Debug)]
pub(crate) struct ProbeRestore {
    original: [u32; BARS],
    modified: u8,
    command: u16,
    phase: ProbeRestorePhase,
}

impl ProbeRestore {
    const fn bars(original: [u32; BARS], modified: u8, command: u16) -> Self {
        Self {
            original,
            modified,
            command,
            phase: ProbeRestorePhase::Bars,
        }
    }

    const fn modified(&self, index: usize) -> bool {
        self.modified & (1 << index) != 0
    }
}

#[derive(PartialEq, Eq, Debug)]
pub(crate) struct RawProbeFailure<E> {
    pub(crate) reason: ProbeError<E>,
    pub(crate) restore: Option<ProbeRestore>,
}

#[derive(PartialEq, Eq, Debug)]
pub(crate) struct RawStagedProbe {
    bars: ProbedBars,
    restore: ProbeRestore,
}

/// A refused BAR probe, including exact retry authority when restoration is uncertain.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "an uncertain BAR probe may carry exclusive restoration authority"]
pub struct ProbeFailure<E> {
    reason: ProbeError<E>,
    retry: Option<ProbeRetry>,
}

impl<E> ProbeFailure<E> {
    /// Refuse before a probe reservation exists.
    pub fn unavailable(reason: EnableError) -> Self {
        Self {
            reason: ProbeError::Unavailable(reason),
            retry: None,
        }
    }

    /// Refuse while preserving an existing restoration owner.
    pub fn with_retry(reason: ProbeError<E>, retry: ProbeRetry) -> Self {
        Self {
            reason,
            retry: Some(retry),
        }
    }

    pub const fn reason(&self) -> &ProbeError<E> {
        &self.reason
    }

    /// Recover the same function reservation and captured register words.
    pub fn into_retry(self) -> Option<ProbeRetry> {
        self.retry
    }
}

/// The exclusive right to retry one uncertain BAR restoration.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "an uncertain BAR probe must be retried or retained fail-stopped"]
pub struct ProbeRetry {
    probe: CommandProbe,
    restore: ProbeRestore,
}

/// Probe every BAR with decode verified off; an uncertain restoration returns [`ProbeRetry`].
pub fn probe_bars<const N: usize, E>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, ProbeAccess) -> Result<u32, E>,
) -> Result<StagedProbe, ProbeFailure<E>> {
    if !functions.holds(authority) {
        return Err(ProbeFailure::unavailable(EnableError::Stale));
    }
    probe_bars_with_leases(&mut functions.commands, authority, access)
}

pub(crate) fn probe_bars_with_leases<const N: usize, E>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    mut access: impl FnMut(Address, ProbeAccess) -> Result<u32, E>,
) -> Result<StagedProbe, ProbeFailure<E>> {
    let probe = leases
        .begin_probe(authority)
        .map_err(|reason| ProbeFailure {
            reason: ProbeError::Unavailable(reason),
            retry: None,
        })?;
    let function = probe.0.function.at;
    match probe_bars_unleased(authority.bar_registers(), |event| access(function, event)) {
        Ok(staged) => {
            let RawStagedProbe { mut bars, restore } = staged;
            let command = match leases.stage_probe(authority, probe, restore.command) {
                Ok(command) => command,
                Err(probe) => {
                    return Err(ProbeFailure {
                        reason: ProbeError::Unavailable(EnableError::Stale),
                        retry: Some(ProbeRetry { probe, restore }),
                    });
                }
            };
            bars.function = Some(authority.lease);
            Ok(StagedProbe { bars, command })
        }
        Err(failure) => match failure.restore {
            Some(restore) => Err(ProbeFailure {
                reason: failure.reason,
                retry: Some(ProbeRetry { probe, restore }),
            }),
            None => {
                let reason = failure.reason;
                assert!(
                    leases.finish_probe(authority, probe).is_ok(),
                    "a restored BAR-probe refusal lost its exact reservation"
                );
                Err(ProbeFailure {
                    reason,
                    retry: None,
                })
            }
        },
    }
}

/// Retry a BAR restoration through the same function reservation.
pub fn retry_probe<const N: usize, E>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    retry: ProbeRetry,
    access: impl FnMut(Address, ProbeAccess) -> Result<u32, E>,
) -> Result<(), ProbeFailure<E>> {
    if !functions.holds(authority) {
        return Err(ProbeFailure::with_retry(
            ProbeError::Unavailable(EnableError::Stale),
            retry,
        ));
    }
    retry_probe_with_leases(&mut functions.commands, authority, retry, access)
}

pub(crate) fn retry_probe_with_leases<const N: usize, E>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    retry: ProbeRetry,
    mut access: impl FnMut(Address, ProbeAccess) -> Result<u32, E>,
) -> Result<(), ProbeFailure<E>> {
    if !leases.holds(authority, retry.probe.0) {
        return Err(ProbeFailure {
            reason: ProbeError::Unavailable(EnableError::Stale),
            retry: Some(retry),
        });
    }
    let function = retry.probe.0.function.at;
    match finish_probe_restoration(retry.restore, &mut |event| access(function, event)) {
        Ok(()) => {
            assert!(
                leases.finish_probe(authority, retry.probe).is_ok(),
                "a restored BAR retry lost its exact reservation"
            );
            Ok(())
        }
        Err((reason, restore)) => Err(ProbeFailure {
            reason,
            retry: Some(ProbeRetry {
                probe: retry.probe,
                restore,
            }),
        }),
    }
}

pub(crate) fn probe_bars_unleased<E>(
    registers: usize,
    mut access: impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<RawStagedProbe, RawProbeFailure<E>> {
    if registers > BARS {
        return Err(RawProbeFailure {
            reason: ProbeError::Malformed,
            restore: None,
        });
    }

    let command = access(ProbeAccess::ReadCommand).map_err(|reason| RawProbeFailure {
        reason: ProbeError::Access(reason),
        restore: None,
    })? as u16;
    let mut original = [0u32; BARS];
    for (index, slot) in original.iter_mut().enumerate().take(registers) {
        *slot = access(ProbeAccess::ReadBar(index)).map_err(|reason| RawProbeFailure {
            reason: ProbeError::Access(reason),
            restore: None,
        })?;
    }

    let Some(plan) = bar_slots(registers, &original) else {
        return Err(RawProbeFailure {
            reason: ProbeError::Malformed,
            restore: None,
        });
    };

    disable_decode(original, command, &mut access)?;
    let (masks, modified) = read_masks(&plan, original, command, &mut access)?;
    let restore = ProbeRestore::bars(original, modified, command);
    let restore = match restore_probe_bars(restore, &mut access) {
        Ok(restore) => restore,
        Err((reason, restore)) => {
            return Err(RawProbeFailure {
                reason,
                restore: Some(restore),
            });
        }
    };
    let windows = match decode_windows(&plan, &original, &masks) {
        Ok(windows) => windows,
        Err(reason) => return Err(after_probe_error(reason, &mut access, restore)),
    };
    Ok(RawStagedProbe {
        bars: ProbedBars {
            function: None,
            windows,
        },
        restore,
    })
}

fn disable_decode<E>(
    original: [u32; BARS],
    command: u16,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<(), RawProbeFailure<E>> {
    let decode_off = command & !PROBE_DISABLED;
    let command_restore = ProbeRestore {
        original,
        modified: 0,
        command,
        phase: ProbeRestorePhase::Command,
    };
    if let Err(reason) = access(ProbeAccess::WriteCommand(decode_off)) {
        return Err(after_access_error(reason, access, command_restore));
    }
    match access(ProbeAccess::ReadCommand) {
        Ok(bits) if bits as u16 == decode_off => Ok(()),
        Ok(bits) => Err(after_probe_error(
            ProbeError::Refused(bits as u16),
            access,
            command_restore,
        )),
        Err(reason) => Err(after_access_error(reason, access, command_restore)),
    }
}

fn read_masks<E>(
    plan: &[Option<BarSlots>; BARS],
    original: [u32; BARS],
    command: u16,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<([u32; BARS], u8), RawProbeFailure<E>> {
    let mut masks = [0u32; BARS];
    let mut modified = 0u8;
    for slots in plan.iter().flatten() {
        let index = slots.low;
        modified |= 1 << index;
        if let Err(reason) = access(ProbeAccess::WriteBar(index, u32::MAX)) {
            let restore = ProbeRestore::bars(original, modified, command);
            return Err(after_access_error(reason, access, restore));
        }
        if let Some(high) = slots.high {
            modified |= 1 << high;
            if let Err(reason) = access(ProbeAccess::WriteBar(high, u32::MAX)) {
                let restore = ProbeRestore::bars(original, modified, command);
                return Err(after_access_error(reason, access, restore));
            }
        }

        masks[index] = match access(ProbeAccess::ReadBar(index)) {
            Ok(mask) => mask,
            Err(reason) => {
                let restore = ProbeRestore::bars(original, modified, command);
                return Err(after_access_error(reason, access, restore));
            }
        };
        if let Some(high) = slots.high {
            masks[high] = match access(ProbeAccess::ReadBar(high)) {
                Ok(mask) => mask,
                Err(reason) => {
                    let restore = ProbeRestore::bars(original, modified, command);
                    return Err(after_access_error(reason, access, restore));
                }
            };
        }
    }
    Ok((masks, modified))
}

fn restore_probe_bars<E>(
    mut restore: ProbeRestore,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<ProbeRestore, (ProbeError<E>, ProbeRestore)> {
    if restore.phase == ProbeRestorePhase::Bars {
        let decode_off = restore.command & !PROBE_DISABLED;
        let _ = access(ProbeAccess::WriteCommand(decode_off));
        match access(ProbeAccess::ReadCommand) {
            Ok(bits) if bits as u16 == decode_off => {}
            Ok(_) => return Err((ProbeError::RestoreChanged, restore)),
            Err(reason) => return Err((ProbeError::Restore(reason), restore)),
        }

        for index in (0..BARS).rev() {
            if restore.modified(index) {
                let _ = access(ProbeAccess::WriteBar(index, restore.original[index]));
            }
        }
        for index in (0..BARS).rev() {
            if !restore.modified(index) {
                continue;
            }
            match access(ProbeAccess::ReadBar(index)) {
                Ok(bits) if bits == restore.original[index] => {}
                Ok(_) => return Err((ProbeError::RestoreChanged, restore)),
                Err(reason) => return Err((ProbeError::Restore(reason), restore)),
            }
        }
        restore.phase = ProbeRestorePhase::Command;
    }
    Ok(restore)
}

pub(crate) fn finish_probe_restoration<E>(
    restore: ProbeRestore,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<(), (ProbeError<E>, ProbeRestore)> {
    let restore = restore_probe_bars(restore, access)?;
    let _ = access(ProbeAccess::WriteCommand(restore.command));
    match access(ProbeAccess::ReadCommand) {
        Ok(bits) if bits as u16 == restore.command => Ok(()),
        Ok(_) => Err((ProbeError::RestoreChanged, restore)),
        Err(reason) => Err((ProbeError::Restore(reason), restore)),
    }
}

#[cfg(test)]
pub(crate) fn restore_staged_probe<E>(
    staged: RawStagedProbe,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<ProbedBars, RawProbeFailure<E>> {
    if let Err((reason, restore)) = finish_probe_restoration(staged.restore, access) {
        return Err(RawProbeFailure {
            reason,
            restore: Some(restore),
        });
    }
    Ok(staged.bars)
}

fn bar_slots(registers: usize, original: &[u32; BARS]) -> Option<[Option<BarSlots>; BARS]> {
    let mut plan = [None; BARS];
    let mut indices = 0..registers;
    while let Some(low) = core::iter::Iterator::next(&mut indices) {
        let high = if matches!(
            Bar::decode(original[low], 0)?,
            Bar::Memory { wide: true, .. }
        ) {
            Some(core::iter::Iterator::next(&mut indices)?)
        } else {
            None
        };
        plan[low] = Some(BarSlots { low, high });
    }
    Some(plan)
}

fn after_access_error<E>(
    reason: E,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
    restore: ProbeRestore,
) -> RawProbeFailure<E> {
    after_probe_error(ProbeError::Access(reason), access, restore)
}

fn after_probe_error<E>(
    reason: ProbeError<E>,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
    restore: ProbeRestore,
) -> RawProbeFailure<E> {
    match finish_probe_restoration(restore, access) {
        Ok(()) => RawProbeFailure {
            reason,
            restore: None,
        },
        Err((reason, restore)) => RawProbeFailure {
            reason,
            restore: Some(restore),
        },
    }
}

fn decode_windows<E>(
    plan: &[Option<BarSlots>; BARS],
    original: &[u32; BARS],
    masks: &[u32; BARS],
) -> Result<[Option<Window>; BARS], ProbeError<E>> {
    let mut windows = [None; BARS];
    for slots in plan.iter().flatten() {
        let index = slots.low;
        let high = slots.high.map_or(0, |high| original[high]);
        let mask_high = slots.high.map_or(0, |high| masks[high]);
        let (bar, size) = match Bar::decode(original[index], high).ok_or(ProbeError::Malformed)? {
            Bar::Io { port } => {
                let Some(size) = io_window_size(masks[index]) else {
                    continue;
                };
                (Bar::Io { port }, u64::from(size))
            }
            bar @ Bar::Memory { wide, .. } => {
                let width = if wide {
                    MemoryWidth::Bits64
                } else {
                    MemoryWidth::Bits32
                };
                let Some(size) = memory_window_size(masks[index], mask_high, width) else {
                    continue;
                };
                (bar, size)
            }
            Bar::Absent => {
                let Some(size) = memory_window_size(masks[index], 0, MemoryWidth::Bits32) else {
                    continue;
                };
                (
                    Bar::Memory {
                        address: 0,
                        wide: false,
                        prefetchable: false,
                    },
                    size,
                )
            }
        };

        let base = match bar {
            Bar::Memory { address, .. } => address,
            Bar::Io { port } => u64::from(port),
            Bar::Absent => 0,
        };
        if !size.is_power_of_two() || !base.is_multiple_of(size) {
            return Err(ProbeError::InvalidWindow);
        }
        windows[index] = Some(Window { bar, size });
    }
    Ok(windows)
}
