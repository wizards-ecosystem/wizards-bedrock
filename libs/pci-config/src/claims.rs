use core::sync::atomic::{AtomicU64, Ordering};

use crate::{Address, CommandLeases, HeaderType};

/// The exclusive right to mutate one PCI function.
///
/// Its registry identity, generation and address are opaque. A token minted by
/// another registry therefore cannot authorize a hardware adapter backed by
/// this one, even when both name the same bus/device/function tuple.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "dropping function authority permanently retains its claim"]
pub struct FunctionAuthority {
    pub(crate) lease: FunctionLease,
    pub(crate) registers: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FunctionLease {
    pub(crate) registry: u64,
    pub(crate) at: Address,
    pub(crate) generation: u64,
}

impl FunctionAuthority {
    /// The claimed bus/device/function identity, with register offset zero.
    pub const fn address(&self) -> Address {
        self.lease.at
    }

    /// BAR register slots proved by the claimed function's header layout.
    pub const fn bar_registers(&self) -> usize {
        self.registers
    }
}

/// A bounded registry of exclusive function authorities.
#[derive(PartialEq, Eq, Debug)]
pub struct FunctionClaims<const N: usize> {
    pub(crate) slots: [Option<FunctionLease>; N],
    pub(crate) mapped_bars: [u8; N],
    identity: u64,
    generation: u64,
    pub(crate) commands: CommandLeases<N>,
}

static NEXT_FUNCTION_REGISTRY: AtomicU64 = AtomicU64::new(1);

impl<const N: usize> FunctionClaims<N> {
    /// Create an empty function-authority registry.
    pub const fn new() -> Self {
        Self {
            slots: [None; N],
            mapped_bars: [0; N],
            identity: 0,
            generation: 0,
            commands: CommandLeases::new(),
        }
    }

    /// Claim one function whose header layout was read from that function.
    ///
    /// The returned token is bound to this registry, a non-repeating
    /// generation and the canonical function address.
    ///
    /// # Errors
    ///
    /// Refuses unsupported header layouts, a duplicate function, a full
    /// registry or exhausted non-repeating identities before changing a slot.
    pub fn claim(
        &mut self,
        at: Address,
        header: HeaderType,
    ) -> Result<FunctionAuthority, FunctionClaimError> {
        let Some(registers) = header.bar_registers() else {
            return Err(FunctionClaimError::Header(header.layout()));
        };
        let at = at.function_base();
        if self.is_claimed(at) {
            return Err(FunctionClaimError::Taken);
        }
        let Some(room) = self.slots.iter().position(Option::is_none) else {
            return Err(FunctionClaimError::NoRoom);
        };
        let registry = if self.identity == 0 {
            let identity = NEXT_FUNCTION_REGISTRY
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    current.checked_add(1)
                })
                .map_err(|_| FunctionClaimError::Exhausted)?;
            self.identity = identity;
            identity
        } else {
            self.identity
        };
        let Some(generation) = self.generation.checked_add(1) else {
            return Err(FunctionClaimError::Exhausted);
        };
        self.generation = generation;
        let lease = FunctionLease {
            registry,
            at,
            generation,
        };
        self.slots[room] = Some(lease);
        self.mapped_bars[room] = 0;
        Ok(FunctionAuthority { lease, registers })
    }

    /// Whether `authority` is this registry's exact live generation.
    pub fn holds(&self, authority: &FunctionAuthority) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|held| *held == authority.lease)
    }

    /// Whether any register offset of `at`'s function is held.
    pub fn is_claimed(&self, at: Address) -> bool {
        let at = at.function_base();
        self.slots.iter().flatten().any(|held| held.at == at)
    }

    /// Release one exact function authority.
    ///
    /// # Errors
    ///
    /// Returns the unchanged token if it is foreign or stale.
    pub(crate) fn release(
        &mut self,
        authority: FunctionAuthority,
    ) -> Result<(), FunctionAuthority> {
        let Some(index) = self
            .slots
            .iter()
            .position(|held| *held == Some(authority.lease))
        else {
            return Err(authority);
        };
        if self.mapped_bars.get(index).copied() != Some(0) {
            return Err(authority);
        }
        let Some(slot) = self.slots.get_mut(index) else {
            return Err(authority);
        };
        *slot = None;
        Ok(())
    }

    /// Functions recorded so far.
    #[expect(
        clippy::len_without_is_empty,
        reason = "the authority registry exposes a diagnostic count, not a collection API"
    )]
    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }
}

impl<const N: usize> Default for FunctionClaims<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Why exclusive function authority could not be minted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum FunctionClaimError {
    /// Another authority already holds this function.
    Taken,
    /// The registry's fixed lifetime bound is exhausted.
    NoRoom,
    /// Registry identity or generation space is exhausted.
    Exhausted,
    /// The header layout is neither a device nor a bridge.
    Header(u8),
}

impl core::fmt::Display for FunctionClaimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Taken => core::fmt::Write::write_str(f, "the function is already claimed"),
            Self::NoRoom => core::fmt::Write::write_str(f, "the function registry is full"),
            Self::Exhausted => {
                core::fmt::Write::write_str(f, "function claim generations are exhausted")
            }
            Self::Header(layout) => write!(f, "header layout {layout} is unsupported"),
        }
    }
}

impl core::error::Error for FunctionClaimError {}
