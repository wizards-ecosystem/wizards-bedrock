use core::fmt;

use crate::claims::FunctionLease;
use crate::command_state::{CommandPhase, admit_bus_master_with_leases, change_command};
use crate::{
    Address, Bar, CommandAccess, CommandLeases, CommandState, EnableError, FunctionAuthority,
    FunctionClaims, ProbedBars, Window, command, header,
};

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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarAuthorityError {
    Stale,
    NotMemory,
    Busy,
}

impl fmt::Display for BarAuthorityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("the BAR authority is stale or foreign"),
            Self::NotMemory => formatter.write_str("the BAR is not a memory window"),
            Self::Busy => formatter.write_str("the BAR already has a mapping authority"),
        }
    }
}

impl core::error::Error for BarAuthorityError {}

#[derive(PartialEq, Eq, Debug)]
#[must_use = "a refused BAR release returns its authority for retry"]
pub struct BarAuthorityFailure {
    reason: BarAuthorityError,
    authority: BarAuthority,
}

impl fmt::Display for BarAuthorityFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.reason, formatter)
    }
}

impl core::error::Error for BarAuthorityFailure {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarIdentityError {
    Stale,
    Access,
    Changed,
}

impl fmt::Display for BarIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("the BAR identity is stale or foreign"),
            Self::Access => formatter.write_str("the BAR identity is unreadable"),
            Self::Changed => formatter.write_str("the BAR identity changed"),
        }
    }
}

impl core::error::Error for BarIdentityError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarEnableAccess {
    ReadBar(usize),
    Command(CommandAccess),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarEnableError {
    Identity(BarIdentityError),
    Command(EnableError),
}

impl fmt::Display for BarEnableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity(reason) => write!(formatter, "BAR identity: {reason}"),
            Self::Command(reason) => write!(formatter, "memory decode: {reason}"),
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
    let Some(bits) = access(at, CommandAccess::Read) else {
        return Err(EnableError::Access);
    };
    let current = bits as u16;
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
