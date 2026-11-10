use core::fmt;

use crate::claims::FunctionLease;
use crate::command_state::{CommandPhase, admit_bus_master_with_leases, change_command};
use crate::{
    Address, Bar, CommandAccess, CommandLeases, CommandState, EnableError, FunctionAuthority,
    FunctionClaims, ProbedBars, Window, command, header,
};

/// A live function's exclusive authority over one probed memory BAR.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "a BAR authority must remain with its mapping or be returned"]
pub struct BarAuthority {
    pub(crate) function: FunctionLease,
    pub(crate) index: usize,
    pub(crate) window: Window,
}

impl BarAuthority {
    pub const fn index(&self) -> usize {
        self.index
    }

    /// The exact decoded window observed by the restored probe.
    pub const fn window(&self) -> Window {
        self.window
    }
}

/// Why a probed memory BAR could not become an exclusive mapping authority.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum BarAuthorityError {
    /// The function, registry generation, or probe provenance is not current.
    Stale,
    /// The BAR slot does not contain a memory window.
    NotMemory,
    /// Another live mapping authority already owns this BAR slot.
    Busy,
}

impl fmt::Display for BarAuthorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => f.write_str("the BAR authority is stale or foreign"),
            Self::NotMemory => f.write_str("the BAR is not a memory window"),
            Self::Busy => f.write_str("the BAR already has a mapping authority"),
        }
    }
}

impl core::error::Error for BarAuthorityError {}

/// A refused BAR-authority return with the unchanged token.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "a refused BAR release returns its authority for retry"]
pub struct BarAuthorityFailure {
    reason: BarAuthorityError,
    authority: BarAuthority,
}

impl fmt::Display for BarAuthorityFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.reason, f)
    }
}

impl core::error::Error for BarAuthorityFailure {}

/// Why a live BAR no longer matches the probed identity behind its mapping.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum BarIdentityError {
    /// The function or BAR mapping generation is stale or foreign.
    Stale,
    /// Configuration space could not read the complete BAR identity.
    Access,
    /// The decoded BAR value changed since the restored probe.
    Changed,
}

impl fmt::Display for BarIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => f.write_str("the BAR identity is stale or foreign"),
            Self::Access => f.write_str("the BAR identity is unreadable"),
            Self::Changed => f.write_str("the BAR identity changed"),
        }
    }
}

impl core::error::Error for BarIdentityError {}

/// One access in the indivisible BAR-identity and memory-decode transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarEnableAccess {
    /// Read one BAR slot before any command write.
    ReadBar(usize),
    /// Access the command register through its verified transaction.
    Command(CommandAccess),
}

/// Why BAR-bound memory decoding could not be enabled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum BarEnableError {
    /// The mapped BAR identity was stale, inaccessible, or changed.
    Identity(BarIdentityError),
    /// The command transaction refused or could not restore exactly.
    Command(EnableError),
}

impl fmt::Display for BarEnableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity(reason) => write!(f, "BAR identity: {reason}"),
            Self::Command(reason) => write!(f, "memory decode: {reason}"),
        }
    }
}

impl core::error::Error for BarEnableError {}

impl<const N: usize> FunctionClaims<N> {
    pub(crate) fn slot_of(&self, function: FunctionLease) -> Option<usize> {
        self.slots.iter().position(|held| *held == Some(function))
    }

    fn holds_bar(&self, authority: &BarAuthority) -> bool {
        let Some(slot) = self.slot_of(authority.function) else {
            return false;
        };
        let Some(bit) = 1u8.checked_shl(authority.index as u32) else {
            return false;
        };
        self.mapped_bars
            .get(slot)
            .is_some_and(|mapped| mapped & bit != 0)
    }
}

/// Add bus mastering through one live BAR mapping isolated by a DMA domain for that function.
pub fn admit_mapped_bus_master<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &BarAuthority,
    isolated: Address,
    state: &mut CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if authority.function.at.function_base() != isolated.function_base() {
        return Err(EnableError::ForeignDomain);
    }
    if !functions.holds_bar(authority) {
        return Err(EnableError::Stale);
    }
    admit_bus_master_with_leases(&mut functions.commands, authority.function, state, access)
}

/// Reserve one probed memory BAR under its exact function generation.
pub fn bind_memory_bar<const N: usize>(
    functions: &mut FunctionClaims<N>,
    function: &FunctionAuthority,
    bars: &ProbedBars,
    index: usize,
) -> Result<BarAuthority, BarAuthorityError> {
    if !functions.holds(function) || bars.function != Some(function.lease) {
        return Err(BarAuthorityError::Stale);
    }
    let Some(
        window @ Window {
            bar: Bar::Memory { .. },
            ..
        },
    ) = bars.window(index)
    else {
        return Err(BarAuthorityError::NotMemory);
    };
    let Some(slot) = functions.slot_of(function.lease) else {
        return Err(BarAuthorityError::Stale);
    };
    let Some(bit) = 1u8.checked_shl(index as u32) else {
        return Err(BarAuthorityError::NotMemory);
    };
    let Some(mapped) = functions.mapped_bars.get_mut(slot) else {
        return Err(BarAuthorityError::Stale);
    };
    if *mapped & bit != 0 {
        return Err(BarAuthorityError::Busy);
    }
    *mapped |= bit;
    Ok(BarAuthority {
        function: function.lease,
        index,
        window,
    })
}

/// Return one exact BAR mapping authority to its function registry.
pub fn release_memory_bar<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: BarAuthority,
) -> Result<(), BarAuthorityFailure> {
    let Some(slot) = functions.slot_of(authority.function) else {
        return Err(BarAuthorityFailure {
            reason: BarAuthorityError::Stale,
            authority,
        });
    };
    let Some(bit) = 1u8.checked_shl(authority.index as u32) else {
        return Err(BarAuthorityFailure {
            reason: BarAuthorityError::Stale,
            authority,
        });
    };
    let Some(mapped) = functions.mapped_bars.get_mut(slot) else {
        return Err(BarAuthorityFailure {
            reason: BarAuthorityError::Stale,
            authority,
        });
    };
    if *mapped & bit == 0 {
        return Err(BarAuthorityFailure {
            reason: BarAuthorityError::Stale,
            authority,
        });
    }
    *mapped &= !bit;
    Ok(())
}

/// Re-read a mapped BAR through reads alone and require its decoded identity to be unchanged.
fn verify_memory_bar<const N: usize>(
    functions: &FunctionClaims<N>,
    function: &FunctionAuthority,
    authority: &BarAuthority,
    mut read: impl FnMut(Address, usize) -> Option<u32>,
) -> Result<(), BarIdentityError> {
    if function.lease != authority.function
        || !functions.holds(function)
        || !functions.holds_bar(authority)
    {
        return Err(BarIdentityError::Stale);
    }
    let low_offset = u16::try_from(authority.index)
        .ok()
        .and_then(|index| index.checked_mul(4))
        .and_then(|offset| header::BAR0.checked_add(offset))
        .ok_or(BarIdentityError::Stale)?;
    let low_at = function
        .address()
        .at(low_offset)
        .ok_or(BarIdentityError::Stale)?;
    let low = read(low_at, authority.index).ok_or(BarIdentityError::Access)?;
    let high = match authority.window.bar {
        Bar::Memory { wide: true, .. } => {
            let high_offset = low_offset.checked_add(4).ok_or(BarIdentityError::Stale)?;
            if authority.index + 1 >= function.bar_registers() {
                return Err(BarIdentityError::Stale);
            }
            let high_at = function
                .address()
                .at(high_offset)
                .ok_or(BarIdentityError::Stale)?;
            read(high_at, authority.index + 1).ok_or(BarIdentityError::Access)?
        }
        Bar::Memory { wide: false, .. } => 0,
        Bar::Io { .. } | Bar::Absent => return Err(BarIdentityError::Stale),
    };
    if Bar::decode(low, high) != Some(authority.window.bar) {
        return Err(BarIdentityError::Changed);
    }
    Ok(())
}

/// Enable memory decode after the BAR identity is re-proven, all under one configuration lock.
pub fn enable_memory_bar<const N: usize>(
    functions: &mut FunctionClaims<N>,
    function: &FunctionAuthority,
    authority: &BarAuthority,
    state: &mut CommandState,
    mut access: impl FnMut(Address, BarEnableAccess) -> Option<u32>,
) -> Result<(), BarEnableError> {
    verify_memory_bar(functions, function, authority, |at, index| {
        access(at, BarEnableAccess::ReadBar(index))
    })
    .map_err(BarEnableError::Identity)?;
    admit_memory_space_with_leases(
        &mut functions.commands,
        function.lease,
        state,
        |at, command| access(at, BarEnableAccess::Command(command)),
    )
    .map_err(BarEnableError::Command)
}

fn admit_memory_space_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    function: FunctionLease,
    state: &mut CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if state.phase != CommandPhase::Installed || !leases.holds_function(function, state.lease) {
        return Err(EnableError::Stale);
    }
    let at = state.lease.command_address();
    let current = match access(at, CommandAccess::Read) {
        Some(bits) => bits as u16,
        None => return Err(EnableError::Access),
    };
    if current != state.installed {
        return Err(EnableError::Changed(current));
    }
    let wanted = (current | command::MEMORY_SPACE) & !command::BUS_MASTER;
    state.phase = CommandPhase::Restoring(wanted);
    match change_command(at, current, wanted, &mut access) {
        Ok(()) => {
            state.installed = wanted;
            state.phase = CommandPhase::Installed;
            Ok(())
        }
        Err(EnableError::Restore) => Err(EnableError::Restore),
        Err(reason) => {
            state.phase = CommandPhase::Installed;
            Err(reason)
        }
    }
}
