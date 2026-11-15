use core::fmt;

use handle_space::Rights;
use wost::arch::current::paging::{ActivateError, AddressSpace, MapError};
use wost::arch::current::task;
use wost::arch::current::{cpu, xstate};

use super::call::cross;
use super::load::{LoadError, load, place_stack};
use super::reap::slot_has_zombie;
use super::{
    APART, BOOTSTRAPS, Bootstrap, BootstrapAuthority, LAUNCHES, Launch, RING, Regions, STACK_BYTES,
    Spawned,
};
use crate::handle::{HandleError, HandleTable};
use crate::{object, queue, thread};

#[derive(Debug)]
struct ProcessConstruction {
    space: Option<AddressSpace>,
    authority: thread::SpaceAuthority,
    regions: Regions,
}

impl ProcessConstruction {
    fn new(space: AddressSpace) -> Self {
        Self {
            space: Some(space),
            authority: thread::SpaceAuthority::None,
            regions: Regions::new(),
        }
    }

    fn space(&mut self) -> Result<&mut AddressSpace, SpawnError> {
        self.space.as_mut().ok_or(SpawnError::Incomplete)
    }

    fn remember(&mut self, at: u64, len: u64) -> Result<(), SpawnError> {
        if self.regions.record(at, len) {
            Ok(())
        } else {
            Err(SpawnError::TooManyMappings)
        }
    }

    fn retain_authority(&mut self, authority: thread::SpaceAuthority) {
        self.authority = authority;
    }
}

impl Drop for ProcessConstruction {
    fn drop(&mut self) {
        let released = match core::mem::replace(&mut self.authority, thread::SpaceAuthority::None) {
            thread::SpaceAuthority::None => None,
            thread::SpaceAuthority::Released(released) => Some(released),
        };
        let Some(space) = self.space.as_mut() else {
            return;
        };

        if let Err(refusal) = self.regions.unmap(space) {
            panic!("failed-spawn mapping cleanup refused: {}", refusal.reason);
        }

        if let Err(reason) = space.reclaim(released) {
            panic!("failed-spawn table cleanup refused: {reason}");
        }
    }
}

#[derive(Debug)]
struct PreparedProcess {
    construction: ProcessConstruction,
    table: alloc::sync::Arc<HandleTable>,
    ring: alloc::sync::Arc<queue::Queue>,
    launch: Launch,
}

impl PreparedProcess {
    #[expect(
        clippy::result_large_err,
        reason = "a refused commit hands the process back to the spawn that retries or drops it"
    )]
    fn commit(mut self, slot: usize, bound: task::Bound) -> Result<(), (Self, task::Bound)> {
        if slot_has_zombie(slot) {
            return Err((self, bound));
        }
        assert!(
            LAUNCHES.lock_leaf().get(slot).is_some_and(Option::is_none),
            "a reserved process slot retained launch state"
        );
        assert!(
            BOOTSTRAPS
                .lock_leaf()
                .get(slot)
                .is_some_and(Option::is_none),
            "a reserved process slot retained a live owner"
        );
        let Some(space) = self.construction.space.take() else {
            return Err((self, bound));
        };
        LAUNCHES.lock_leaf()[slot] = Some(self.launch);
        BOOTSTRAPS.lock_leaf()[slot] = Some(Bootstrap {
            space,
            authority: Some(BootstrapAuthority::Bound(bound)),
            regions: core::mem::replace(&mut self.construction.regions, Regions::new()),
            table: self.table,
            ring: self.ring,
        });
        Ok(())
    }
}

const HANDLE_QUOTA: usize = 64;

#[derive(Debug)]
pub(crate) enum SpawnError {
    NoInterruptAdmission,
    NoUserIsolation,
    NoExecuteProtection,
    NoExtendedState,
    NoClock,
    NoServiceWorker,
    UnguardedBootStack,
    NoSpace(MapError),
    Load(LoadError),
    NoStack,
    NotBindable(ActivateError),
    NoThread(thread::SpawnError),
    Incomplete,
    TooManyMappings,
    NoBootstrap(HandleError),
    NoTimePage(crate::timepage::Error),
    NoRing,
}

impl fmt::Display for SpawnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoInterruptAdmission => {
                formatter.write_str("maskable interrupts are not admitted; ring 3 is closed")
            }
            Self::NoUserIsolation => {
                formatter.write_str("no admitted user isolation mode; ring 3 is closed")
            }
            Self::NoExecuteProtection => {
                formatter.write_str("no execute protection; ring 3 is closed")
            }
            Self::NoExtendedState => {
                formatter.write_str("no isolated extended state; ring 3 is closed")
            }
            Self::NoClock => formatter.write_str("no calibrated clock; ring 3 is closed"),
            Self::NoServiceWorker => formatter.write_str("no service worker; ring 3 is closed"),
            Self::UnguardedBootStack => {
                formatter.write_str("the boot thread has no guarded stack; ring 3 is closed")
            }
            Self::NoSpace(reason) => write!(formatter, "no address space: {reason}"),
            Self::Load(reason) => write!(formatter, "{reason}"),
            Self::NoStack => formatter.write_str("no memory for a stack"),
            Self::NotBindable(reason) => {
                write!(formatter, "the space cannot be installed: {reason}")
            }
            Self::NoThread(reason) => write!(formatter, "no thread: {reason}"),
            Self::Incomplete => formatter.write_str("the process construction is incomplete"),
            Self::TooManyMappings => formatter.write_str("the process has too many owned mappings"),
            Self::NoBootstrap(reason) => write!(formatter, "no bootstrap channel: {reason}"),
            Self::NoTimePage(reason) => write!(formatter, "the time page: {reason}"),
            Self::NoRing => formatter.write_str("no memory for a ring"),
        }
    }
}

impl core::error::Error for SpawnError {}

pub(crate) fn spawn(name: &'static str, which: u64, argument: u64) -> Result<Spawned, SpawnError> {
    let mut construction = ProcessConstruction::new(admitted_space()?);
    let (entry, stack) = map_program(&mut construction, which)?;
    let ring = map_shared(&mut construction)?;
    let root = construction.space()?.root();

    // Create the bootstrap identity before the thread can observe it.
    let bootstrap = object::bootstrap_identity();
    let table = HandleTable::new(HANDLE_QUOTA).map_err(SpawnError::NoBootstrap)?;
    #[cfg(feature = "selftest")]
    let table_identity = table.identity_for_test();
    let handle = table
        .insert(bootstrap, Rights::UNIVERSAL)
        .map_err(SpawnError::NoBootstrap)?;

    let binding = construction
        .space()?
        .bind()
        .map_err(SpawnError::NotBindable)?;

    let mut prepared = Some(PreparedProcess {
        construction,
        table: alloc::sync::Arc::new(table),
        ring,
        launch: Launch {
            entry,
            stack,
            argument,
            bootstrap_handle: handle,
            which,
            root,
        },
    });
    let arrange = |slot, bound| {
        if slot >= thread::CAPACITY {
            return Err(bound);
        }
        let Some(process) = prepared.take() else {
            return Err(bound);
        };
        process.commit(slot, bound).map_err(|(process, bound)| {
            prepared = Some(process);
            bound
        })
    };
    let slot = match thread::spawn_in_space(name, cross, binding, arrange) {
        Ok(slot) => slot,
        Err(failure) => {
            if let Some(process) = prepared.as_mut() {
                process.construction.retain_authority(failure.authority);
            }
            return Err(SpawnError::NoThread(failure.reason));
        }
    };

    Ok(Spawned {
        slot,
        #[cfg(feature = "selftest")]
        observed_table_incarnation: table_identity,
    })
}

fn admitted_space() -> Result<AddressSpace, SpawnError> {
    if !crate::maskable_interrupts_are_ready() {
        return Err(SpawnError::NoInterruptAdmission);
    }
    if !crate::user_launch_is_safe() {
        return Err(SpawnError::UnguardedBootStack);
    }
    let isolation = crate::user_isolation();
    if !cpu::Protections::enabled().nx {
        return Err(SpawnError::NoExecuteProtection);
    }
    if !xstate::user_state_is_isolated() {
        return Err(SpawnError::NoExtendedState);
    }
    if !crate::time::is_calibrated() {
        return Err(SpawnError::NoClock);
    }
    if !crate::queue::worker_is_ready() {
        return Err(SpawnError::NoServiceWorker);
    }
    match isolation {
        crate::UserIsolation::Closed => return Err(SpawnError::NoUserIsolation),
        crate::UserIsolation::Shared => AddressSpace::for_user(),
        crate::UserIsolation::Paired => AddressSpace::for_user_isolated(),
    }
    .map_err(SpawnError::NoSpace)
}

fn map_program(
    construction: &mut ProcessConstruction,
    which: u64,
) -> Result<(u64, u64), SpawnError> {
    let image = load(construction.space()?, which * APART).map_err(SpawnError::Load)?;
    construction.regions = image.regions;

    let stack = place_stack(construction.space()?, which).ok_or(SpawnError::NoStack)?;
    construction.remember(stack, STACK_BYTES)?;
    Ok((image.entry, stack))
}

// The ring's pages stay the ring's and never enter the cleanup ledger.
fn map_shared(
    construction: &mut ProcessConstruction,
) -> Result<alloc::sync::Arc<queue::Queue>, SpawnError> {
    crate::timepage::install(construction.space()?).map_err(SpawnError::NoTimePage)?;

    let mut ring = queue::Queue::new().ok_or(SpawnError::NoRing)?;
    alloc::sync::Arc::get_mut(&mut ring)
        .ok_or(SpawnError::Incomplete)?
        .install(construction.space()?, RING)
        .map_err(SpawnError::NoSpace)?;
    Ok(ring)
}
