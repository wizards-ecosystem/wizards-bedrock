use core::fmt;

use crate::claims::FunctionLease;
use crate::{Address, CommandLease, CommandLeases, FunctionAuthority, FunctionClaims, command};

/// One command-register access in a verified enable transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandAccess {
    /// Only the low sixteen bits are consumed.
    Read,
    /// Write only command bits, leaving the status half of the dword clear.
    Write(u16),
}

/// The exclusive right to finish one function's command-register transition.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "dropping command state permanently retains its function lease"]
pub struct CommandState {
    pub(crate) lease: CommandLease,
    pub(crate) before: u16,
    pub(crate) installed: u16,
    pub(crate) phase: CommandPhase,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CommandPhase {
    Installed,
    Restoring(u16),
}

/// Why one function authority could not be returned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum FunctionReleaseError {
    /// A command transition or BAR restoration still owns this function.
    Active,
    /// The token is foreign to this function registry or no longer current.
    Stale,
}

impl fmt::Display for FunctionReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => f.write_str("the function still has a configuration sublease"),
            Self::Stale => f.write_str("the function authority is foreign or stale"),
        }
    }
}

impl core::error::Error for FunctionReleaseError {}

/// A refused function release with unchanged retry authority.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "a refused function release returns its authority for retry"]
pub struct FunctionReleaseFailure {
    reason: FunctionReleaseError,
    authority: FunctionAuthority,
}

impl FunctionReleaseFailure {
    pub const fn reason(&self) -> FunctionReleaseError {
        self.reason
    }

    pub fn into_authority(self) -> FunctionAuthority {
        self.authority
    }
}

impl fmt::Display for FunctionReleaseFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.reason, f)
    }
}

impl core::error::Error for FunctionReleaseFailure {}

/// Release a function only after every command and probe sublease is gone.
pub fn release_function<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: FunctionAuthority,
) -> Result<(), FunctionReleaseFailure> {
    if !functions.holds(&authority) {
        return Err(FunctionReleaseFailure {
            reason: FunctionReleaseError::Stale,
            authority,
        });
    }
    if functions.commands.has_function(&authority) {
        return Err(FunctionReleaseFailure {
            reason: FunctionReleaseError::Active,
            authority,
        });
    }
    if functions
        .slot_of(authority.lease)
        .and_then(|slot| functions.mapped_bars.get(slot))
        .is_some_and(|mapped| *mapped != 0)
    {
        return Err(FunctionReleaseFailure {
            reason: FunctionReleaseError::Active,
            authority,
        });
    }
    functions
        .release(authority)
        .map_err(|authority| FunctionReleaseFailure {
            reason: FunctionReleaseError::Stale,
            authority,
        })
}

/// Exclusive command-register reservation held for one BAR probe.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "dropping a probe token permanently retains its function reservation"]
pub struct CommandProbe(pub(crate) CommandLease);

/// A refused command enablement, including cleanup authority when uncertain.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "a refused command transition may retain restoration authority"]
pub struct EnableFailure {
    reason: EnableError,
    state: Option<CommandState>,
}

impl EnableFailure {
    pub const fn reason(&self) -> EnableError {
        self.reason
    }

    pub fn into_state(self) -> Option<CommandState> {
        self.state
    }
}

impl From<EnableError> for EnableFailure {
    fn from(reason: EnableError) -> Self {
        Self {
            reason,
            state: None,
        }
    }
}

/// A refused command restoration with the unchanged retry state.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "a refused command restoration returns its state for retry"]
pub struct RestoreFailure {
    reason: EnableError,
    state: CommandState,
}

impl RestoreFailure {
    pub const fn reason(&self) -> EnableError {
        self.reason
    }

    pub fn into_state(self) -> CommandState {
        self.state
    }
}

/// Why memory decode and bus mastering could not be verified.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum EnableError {
    /// This function already has a live command transition.
    Leased,
    /// Every bounded command-transition slot is occupied.
    NoRoom,
    /// The non-repeating command-transition generation was exhausted.
    Exhausted,
    /// The token does not name a live registry generation.
    Stale,
    /// The command word changed after this lease installed it.
    Changed(u16),
    /// A configuration access failed and any attempted change was restored.
    Access,
    /// The device read back a command other than the exact word written.
    Refused(u16),
    /// The original command could not be read back exactly after failure.
    Restore,
    /// The DMA domain offered as isolation names another function.
    ForeignDomain,
}

impl fmt::Display for EnableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Leased => f.write_str("the function already has a command lease"),
            Self::NoRoom => f.write_str("the command-lease registry is full"),
            Self::Exhausted => f.write_str("command-lease generations are exhausted"),
            Self::Stale => f.write_str("the command lease is no longer current"),
            Self::Changed(observed) => {
                write!(f, "the leased command register changed to {observed:#06x}")
            }
            Self::Access => f.write_str("configuration access failed"),
            Self::Refused(observed) => {
                write!(f, "the command register read back as {observed:#06x}")
            }
            Self::Restore => f.write_str("the original command could not be verified"),
            Self::ForeignDomain => f.write_str("the DMA domain isolates another function"),
        }
    }
}

impl core::error::Error for EnableError {}

/// Enable memory decode while explicitly withholding bus mastering.
pub fn enable_memory_space<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    if !functions.holds(authority) {
        return Err(EnableFailure::from(EnableError::Stale));
    }
    enable_memory_space_with_leases(&mut functions.commands, authority, access)
}

/// Fence one requester's bus mastering for a unit-wide transition; restoring it never revives BME.
pub fn fence_bus_master<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    if !functions.holds(authority) {
        return Err(EnableFailure::from(EnableError::Stale));
    }
    let mut state = begin_command(
        &mut functions.commands,
        authority,
        0,
        command::BUS_MASTER,
        access,
    )?;
    state.before &= !command::BUS_MASTER;
    Ok(state)
}

/// Freshly verify that one live command owner still has BME clear.
pub fn verify_bus_master_fence<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    state: &CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if !functions.holds(authority)
        || !functions.commands.holds(authority, state.lease)
        || state.phase != CommandPhase::Installed
    {
        return Err(EnableError::Stale);
    }
    let current = access(state.lease.command_address(), CommandAccess::Read)
        .ok_or(EnableError::Access)? as u16;
    if current != state.installed {
        return Err(EnableError::Changed(current));
    }
    if current & command::BUS_MASTER != 0 {
        return Err(EnableError::Refused(current));
    }
    Ok(())
}

pub(crate) fn enable_memory_space_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    begin_command(
        leases,
        authority,
        command::MEMORY_SPACE,
        command::BUS_MASTER,
        access,
    )
}

#[cfg(test)]
pub(crate) fn enable_bus_master<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    if !functions.holds(authority) {
        return Err(EnableFailure::from(EnableError::Stale));
    }
    enable_bus_master_with_leases(&mut functions.commands, authority, access)
}

#[cfg(test)]
pub(crate) fn enable_bus_master_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    begin_command(
        leases,
        authority,
        command::MEMORY_SPACE | command::BUS_MASTER,
        0,
        access,
    )
}

fn begin_command<const N: usize>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    set: u16,
    clear: u16,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableFailure> {
    let lease = leases.reserve(authority).map_err(|reason| EnableFailure {
        reason,
        state: None,
    })?;
    let at = lease.command_address();
    let before = match access(at, CommandAccess::Read) {
        Some(bits) => bits as u16,
        None => {
            assert!(
                leases.release(lease),
                "a pre-write command refusal lost its exact reservation"
            );
            return Err(EnableFailure {
                reason: EnableError::Access,
                state: None,
            });
        }
    };
    let installed = (before | set) & !clear;
    let mut state = CommandState {
        lease,
        before,
        installed,
        phase: CommandPhase::Restoring(before),
    };
    match change_command(at, before, installed, &mut access) {
        Ok(()) => {
            state.phase = CommandPhase::Installed;
            Ok(state)
        }
        Err(reason) if reason == EnableError::Restore => Err(EnableFailure {
            reason,
            state: Some(state),
        }),
        Err(reason) => {
            assert!(
                leases.release(lease),
                "a restored command refusal lost its exact reservation"
            );
            Err(EnableFailure {
                reason,
                state: None,
            })
        }
    }
}

pub(crate) fn change_command(
    at: Address,
    original: u16,
    wanted: u16,
    access: &mut impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if original == wanted {
        return Ok(());
    }

    let failure = match access(at, CommandAccess::Write(wanted)) {
        None => EnableError::Access,
        Some(_) => match access(at, CommandAccess::Read) {
            Some(bits) if bits as u16 == wanted => return Ok(()),
            Some(bits) => EnableError::Refused(bits as u16),
            None => EnableError::Access,
        },
    };

    let _ = access(at, CommandAccess::Write(original));
    match access(at, CommandAccess::Read) {
        Some(bits) if bits as u16 == original => Err(failure),
        Some(_) | None => Err(EnableError::Restore),
    }
}

#[cfg(test)]
pub(crate) fn admit_bus_master<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    state: &mut CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if !functions.holds(authority) {
        return Err(EnableError::Stale);
    }
    admit_bus_master_with_leases(&mut functions.commands, authority.lease, state, access)
}

pub(crate) fn admit_bus_master_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    function: FunctionLease,
    state: &mut CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if !leases.holds_function(function, state.lease) || state.phase != CommandPhase::Installed {
        return Err(EnableError::Stale);
    }
    let at = state.lease.command_address();
    let current = access(at, CommandAccess::Read).ok_or(EnableError::Access)? as u16;
    if current != state.installed {
        return Err(EnableError::Changed(current));
    }
    let wanted = current | command::MEMORY_SPACE | command::BUS_MASTER;
    state.phase = CommandPhase::Restoring(wanted);
    match change_command(at, current, wanted, &mut access) {
        Ok(()) => {
            state.installed = wanted;
            state.phase = CommandPhase::Installed;
            Ok(())
        }
        Err(reason) => {
            if reason != EnableError::Restore {
                state.phase = CommandPhase::Installed;
            }
            Err(reason)
        }
    }
}

/// Permanently withhold bus mastering, accepting an interrupted change only at its recorded words.
pub fn disable_bus_master_for_retirement<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    state: &mut CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if !functions.holds(authority) {
        return Err(EnableError::Stale);
    }
    disable_bus_master_for_retirement_with_leases(
        &mut functions.commands,
        authority.lease,
        state,
        access,
    )
}

// `installed` takes the current word before the write so an interrupted retry names both words.
pub(crate) fn disable_bus_master_for_retirement_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    function: FunctionLease,
    state: &mut CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    if !leases.holds_function(function, state.lease) {
        return Err(EnableError::Stale);
    }
    let at = state.lease.command_address();
    let current = access(at, CommandAccess::Read).ok_or(EnableError::Access)? as u16;
    let pending = match state.phase {
        CommandPhase::Installed => None,
        CommandPhase::Restoring(word) => Some(word),
    };
    if current != state.installed && pending != Some(current) {
        return Err(EnableError::Changed(current));
    }

    state.installed = current;
    let wanted = current & !command::BUS_MASTER;
    state.phase = CommandPhase::Restoring(wanted);
    match change_command(at, current, wanted, &mut access) {
        Ok(()) => {
            state.before &= !command::BUS_MASTER;
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

/// Restore the exact command word carried by a [`CommandState`].
pub fn restore_command<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    state: CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), RestoreFailure> {
    if !functions.holds(authority) {
        return Err(RestoreFailure {
            reason: EnableError::Stale,
            state,
        });
    }
    restore_command_with_leases(&mut functions.commands, authority, state, access)
}

pub(crate) fn restore_command_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    mut state: CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), RestoreFailure> {
    if !leases.holds(authority, state.lease) {
        return Err(RestoreFailure {
            reason: EnableError::Stale,
            state,
        });
    }
    let at = state.lease.command_address();
    let current = match access(at, CommandAccess::Read) {
        Some(bits) => bits as u16,
        None => {
            return Err(RestoreFailure {
                reason: EnableError::Access,
                state,
            });
        }
    };
    if current == state.before {
        return if leases.release(state.lease) {
            Ok(())
        } else {
            Err(RestoreFailure {
                reason: EnableError::Stale,
                state,
            })
        };
    }
    let pending = match state.phase {
        CommandPhase::Installed => None,
        CommandPhase::Restoring(word) => Some(word),
    };
    if current != state.installed && pending != Some(current) {
        return Err(RestoreFailure {
            reason: EnableError::Changed(current),
            state,
        });
    }

    state.phase = CommandPhase::Restoring(current);
    let _ = access(at, CommandAccess::Write(state.before));
    match access(at, CommandAccess::Read) {
        Some(bits) if bits as u16 == state.before => {
            if leases.release(state.lease) {
                Ok(())
            } else {
                Err(RestoreFailure {
                    reason: EnableError::Stale,
                    state,
                })
            }
        }
        Some(bits) => Err(RestoreFailure {
            reason: EnableError::Refused(bits as u16),
            state,
        }),
        None => Err(RestoreFailure {
            reason: EnableError::Access,
            state,
        }),
    }
}

/// Finish a permanent command transition without restoring its old word.
pub fn commit_command<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    state: CommandState,
    access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), RestoreFailure> {
    if !functions.holds(authority) {
        return Err(RestoreFailure {
            reason: EnableError::Stale,
            state,
        });
    }
    commit_command_with_leases(&mut functions.commands, authority, state, access)
}

pub(crate) fn commit_command_with_leases<const N: usize>(
    leases: &mut CommandLeases<N>,
    authority: &FunctionAuthority,
    state: CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), RestoreFailure> {
    if !leases.holds(authority, state.lease) || state.phase != CommandPhase::Installed {
        return Err(RestoreFailure {
            reason: EnableError::Stale,
            state,
        });
    }
    let current = match access(state.lease.command_address(), CommandAccess::Read) {
        Some(bits) => bits as u16,
        None => {
            return Err(RestoreFailure {
                reason: EnableError::Access,
                state,
            });
        }
    };
    if current != state.installed {
        return Err(RestoreFailure {
            reason: EnableError::Changed(current),
            state,
        });
    }
    if leases.release(state.lease) {
        Ok(())
    } else {
        Err(RestoreFailure {
            reason: EnableError::Stale,
            state,
        })
    }
}
