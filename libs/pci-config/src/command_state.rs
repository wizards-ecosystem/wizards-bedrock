use crate::claims::FunctionLease;
use crate::{Address, CommandLease, CommandLeases, FunctionAuthority, FunctionClaims, command};

/// One command-register access in a verified enable transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandAccess {
    /// Read the command register. Only its low sixteen bits are consumed.
    Read,
    /// Write only command bits, leaving the status half of the dword clear.
    Write(u16),
}

/// The exclusive right to finish one function's command-register transition.
///
/// The function identity, original word, installed word and registry
/// generation are deliberately opaque. Restoration therefore cannot be aimed
/// at another function or replayed over a later lease.
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

impl core::fmt::Display for FunctionReleaseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Active => {
                core::fmt::Write::write_str(f, "the function still has a configuration sublease")
            }
            Self::Stale => {
                core::fmt::Write::write_str(f, "the function authority is foreign or stale")
            }
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
    /// Why release refused.
    pub const fn reason(&self) -> FunctionReleaseError {
        self.reason
    }

    /// Recover the exact function authority for retry.
    pub fn into_authority(self) -> FunctionAuthority {
        self.authority
    }
}

impl core::fmt::Display for FunctionReleaseFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(&self.reason, f)
    }
}

impl core::error::Error for FunctionReleaseFailure {}

/// Release a function only after every command and probe sublease is gone.
///
/// # Errors
///
/// Returns the unchanged authority while a sublease remains or if the token
/// is foreign or stale. No registry slot changes on refusal.
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
    /// Why enablement refused.
    pub const fn reason(&self) -> EnableError {
        self.reason
    }

    /// Recover an uncertain transition for exact restoration.
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
    /// Why the lease could not be completed.
    pub const fn reason(&self) -> EnableError {
        self.reason
    }

    /// Recover the exact command word for another restoration attempt.
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

impl core::fmt::Display for EnableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Leased => {
                core::fmt::Write::write_str(f, "the function already has a command lease")
            }
            Self::NoRoom => core::fmt::Write::write_str(f, "the command-lease registry is full"),
            Self::Exhausted => {
                core::fmt::Write::write_str(f, "command-lease generations are exhausted")
            }
            Self::Stale => core::fmt::Write::write_str(f, "the command lease is no longer current"),
            Self::Changed(observed) => {
                write!(f, "the leased command register changed to {observed:#06x}")
            }
            Self::Access => core::fmt::Write::write_str(f, "configuration access failed"),
            Self::Refused(observed) => {
                write!(f, "the command register read back as {observed:#06x}")
            }
            Self::Restore => {
                core::fmt::Write::write_str(f, "the original command could not be verified")
            }
            Self::ForeignDomain => {
                core::fmt::Write::write_str(f, "the DMA domain isolates another function")
            }
        }
    }
}

impl core::error::Error for EnableError {}

/// Enable memory decode while explicitly withholding bus mastering.
///
/// This is the safe first stage for a memory-mapped controller whose firmware
/// DMA state must be disabled before bus mastering is admitted.
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

/// Exclusively fence one requester's bus mastering for a unit-wide DMA
/// transition.
///
/// The returned command owner records every captured bit except BME as its
/// restoration target. It must therefore remain attached to the remapping
/// unit until the unit transition is complete; restoring it can never revive
/// unisolated DMA.
///
/// # Errors
///
/// See [`EnableError`]. An uncertain rollback returns its still-live
/// [`CommandState`] through [`EnableFailure`] for quarantine or exact retry.
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
    // `begin_command` verified the installed word exactly. From this point a
    // later restoration is deliberately safe rather than firmware-exact.
    state.before &= !command::BUS_MASTER;
    Ok(state)
}

/// Freshly verify that one live command owner still has BME clear.
///
/// This is the last selected-function check before a unit-wide VT-d write.
/// It neither changes nor consumes the command owner.
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
    let result = change_command(at, current, wanted, &mut access);
    match result {
        Ok(()) => {
            state.installed = wanted;
            state.phase = CommandPhase::Installed;
            Ok(())
        }
        Err(error) => {
            if error != EnableError::Restore {
                state.phase = CommandPhase::Installed;
            }
            Err(error)
        }
    }
}

/// Permanently withhold bus mastering from a transient function lease.
///
/// The command word is read and the BME-clear result is verified while the
/// exact function and command generations remain live. Only a successful
/// readback removes BME from the word a later [`restore_command`] will write;
/// every other captured bit is still restored normally. An interrupted prior
/// command change is accepted only at one of its two recorded words, which
/// lets teardown settle an uncertain bus-master admission without inventing a
/// third state.
///
/// # Errors
///
/// Refuses stale authority, an independently changed word, configuration
/// access failure, or a transition whose exact result cannot be verified. The
/// caller retains `state` for retry or quarantine in every case.
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

    // `current` is now one of the command owner's exact recorded words. Make
    // it the rollback word before publishing the next possible hardware word,
    // so an interrupted retry still names both and only both possibilities.
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
        Err(error) => {
            state.phase = CommandPhase::Installed;
            Err(error)
        }
    }
}

/// Restore the exact command word carried by a [`CommandState`].
///
/// # Errors
///
/// The unchanged state is returned in [`RestoreFailure`] whenever its registry
/// generation is stale, the installed word changed, an access failed, or the
/// final readback did not equal the captured word.
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
    state: CommandState,
    mut access: impl FnMut(Address, CommandAccess) -> Option<u32>,
) -> Result<(), RestoreFailure> {
    let mut state = state;
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
///
/// # Errors
///
/// Returns the unchanged state unless its exact registry generation is live
/// and the command register still equals the installed word.
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
