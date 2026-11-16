use core::fmt;

use native_abi::Rights;
use wost::arch::current::paging::{ActivateError, AddressSpace, MapError};
use wost::arch::current::{cpu, xstate};
use wost::mm::pool::{self as frames, Earmark, Refused};

use super::call::cross;
use super::load::{LoadError, load, place_stack};
use super::{Launch, Regions, STACK_BYTES, Spawned};
use crate::handle::{HandleError, Table, TableRef};
use crate::process::{Authority, Objects, ObservationRef, Process, ProcessRef, Space};
use crate::queue::{self, Queue, QueueRef};
use crate::world::{self, registry};
use crate::{object, thread};

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
pub(crate) enum SpawnError {
    NoInterruptAdmission,
    NoUserIsolation,
    NoExecuteProtection,
    UntrappedCounter,
    NoExtendedState,
    NoClock,
    NoRandomness,
    NoServiceWorker,
    UnguardedBootStack,
    NoSpace(MapError),
    Load(LoadError),
    NoStack,
    NotBindable(ActivateError),
    NoThread(thread::SpawnError),
    Incomplete,
    NoBootstrap(HandleError),
    NoTimePage(crate::timepage::Error),
    NoRing,
    NoWorld,
    NoMemory(Refused),
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
            Self::UntrappedCounter => {
                formatter.write_str("the counter is not trapped; ring 3 is closed")
            }
            Self::NoExtendedState => {
                formatter.write_str("no isolated extended state; ring 3 is closed")
            }
            Self::NoClock => formatter.write_str("no calibrated clock; ring 3 is closed"),
            Self::NoRandomness => {
                formatter.write_str("the random generator is not seeded; ring 3 is closed")
            }
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
            Self::NoBootstrap(reason) => write!(formatter, "no bootstrap channel: {reason}"),
            Self::NoTimePage(reason) => write!(formatter, "the time page: {reason}"),
            Self::NoRing => formatter.write_str("no memory for a ring"),
            Self::NoWorld => formatter.write_str("no root world; ring 3 is closed"),
            Self::NoMemory(reason) => write!(formatter, "the world's arena refused: {reason}"),
        }
    }
}

impl core::error::Error for SpawnError {}

pub(crate) fn spawn(
    context: &mut thread::ThreadContext,
    name: &'static str,
    which: u64,
    argument: u64,
    operand: u64,
) -> Result<Spawned, SpawnError> {
    // The one world every process belongs to until worlds are objects.
    let clock = crate::timepage::root();
    let mut construction = ProcessConstruction::new(admitted_space()?);
    let world_ref = world::root().ok_or(SpawnError::NoWorld)?;
    let world = registry::hold(world_ref).ok_or(SpawnError::NoWorld)?;
    let mut earmark = earmark(context, &world)?;
    let (entry, stack) = map_program(&mut construction, which)?;
    clock
        .install(construction.space()?)
        .map_err(SpawnError::NoTimePage)?;
    let mut queue = Queue::new(clock, &mut earmark).ok_or(SpawnError::NoRing)?;
    queue
        .install(construction.space()?)
        .map_err(SpawnError::NoSpace)?;
    let root = construction.space()?.root();

    let objects = Objects {
        ring: QueueRef::create(&mut earmark, world_ref, &world, queue),
        table: TableRef::create(&mut earmark, world_ref, &world),
    };
    // Created before the thread can observe the table it names.
    let bootstrap = object::bootstrap_identity(&mut earmark, &world);
    let (_, table) = objects.table.hold().ok_or(SpawnError::NoWorld)?;
    let handle = match table.insert(context, bootstrap, Rights::UNIVERSAL) {
        Ok(handle) => handle,
        Err(reason) => {
            bootstrap.release(&world);
            return Err(SpawnError::NoBootstrap(reason));
        }
    };
    drop(table);
    let binding = construction
        .space()?
        .bind()
        .map_err(SpawnError::NotBindable)?;

    let launch = Launch {
        entry,
        stack,
        argument,
        operand,
        bootstrap_handle: handle,
        which,
        root,
    };
    let observation = ObservationRef::create(&mut earmark, world_ref, &world);
    let process = ProcessRef::create(
        &mut earmark,
        world_ref,
        &world,
        Process::new(launch, observation.local(), objects),
    );
    // The process takes the space only once its thread is bound to it.
    let arrange = |slot, bound| {
        let Some(space) = construction.space.take() else {
            return Err(bound);
        };
        let Some((_, held)) = process.hold() else {
            construction.space = Some(space);
            return Err(bound);
        };
        held.commit(
            slot,
            Space {
                space,
                authority: Some(Authority::Bound(bound)),
                regions: core::mem::replace(&mut construction.regions, Regions::new()),
            },
        );
        Ok(())
    };
    let slot = match thread::spawn_in_space(name, cross, binding, clock, process, arrange) {
        Ok(slot) => slot,
        Err(failure) => {
            construction.retain_authority(failure.authority);
            assert!(process.destroy(), "a process that never ran was borrowed");
            return Err(SpawnError::NoThread(failure.reason));
        }
    };

    Ok(Spawned { slot, observation })
}

// Every arena frame the process's kernel objects can need, taken before anything is built.
fn earmark(
    context: &mut thread::ThreadContext,
    world: &world::World,
) -> Result<Earmark, SpawnError> {
    let records = world::ProcessPool::frames_for(1)
        .zip(world::ObservationPool::frames_for(1))
        .and_then(|(process, observation)| process.checked_add(observation));
    let frames = world::TablePool::frames_for(1)
        .zip(QueueRef::frames_for())
        .zip(world::ChannelPool::frames_for(1))
        .zip(records)
        .and_then(|(((table, queue), channel), records)| {
            table
                .checked_add(Table::frames_for(world.handles()))?
                .checked_add(queue)?
                .checked_add(channel)?
                .checked_add(records)
        })
        .ok_or(SpawnError::NoMemory(Refused::NoMemory))?;
    frames::earmark(context, world.arena(), frames).map_err(SpawnError::NoMemory)
}

fn admitted_space() -> Result<AddressSpace, SpawnError> {
    if !crate::interrupts::maskable_interrupts_are_ready() {
        return Err(SpawnError::NoInterruptAdmission);
    }
    if !crate::user_launch_is_safe() {
        return Err(SpawnError::UnguardedBootStack);
    }
    let isolation = crate::interrupts::user_isolation();
    if !cpu::Protections::enabled().nx {
        return Err(SpawnError::NoExecuteProtection);
    }
    if !cpu::counter_is_trapped() {
        return Err(SpawnError::UntrappedCounter);
    }
    if !xstate::user_state_is_isolated() {
        return Err(SpawnError::NoExtendedState);
    }
    if !crate::time::is_calibrated() {
        return Err(SpawnError::NoClock);
    }
    if !crate::random::is_ready() {
        return Err(SpawnError::NoRandomness);
    }
    if !queue::worker_is_ready() {
        return Err(SpawnError::NoServiceWorker);
    }
    match isolation {
        crate::interrupts::UserIsolation::Closed => return Err(SpawnError::NoUserIsolation),
        crate::interrupts::UserIsolation::Shared => AddressSpace::for_user(),
        crate::interrupts::UserIsolation::Paired => AddressSpace::for_user_isolated(),
    }
    .map_err(SpawnError::NoSpace)
}

fn map_program(
    construction: &mut ProcessConstruction,
    which: u64,
) -> Result<(u64, u64), SpawnError> {
    let image = load(construction.space()?).map_err(SpawnError::Load)?;
    construction.regions = image.regions;

    let stack = place_stack(construction.space()?, which).ok_or(SpawnError::NoStack)?;
    assert!(
        construction.regions.record(stack, STACK_BYTES),
        "the region ledger keeps one range beyond the loader's for the stack"
    );
    Ok((image.entry, stack))
}
