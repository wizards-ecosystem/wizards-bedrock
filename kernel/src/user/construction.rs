use core::fmt;

use native_abi::Rights;
use world_tree::NodeRef;
use wost::arch::current::paging::{ActivateError, AddressSpace, MapError, UserAccess};
use wost::arch::current::{cpu, xstate};
use wost::mm::pool::{self as frames, Earmark, Refused};
use x86_paging::PAGE_SIZE;

use super::call::cross;
use super::load::{self, LoadError, Plan};
use super::{Launch, STACK_BYTES, Spawned};
use crate::handle::{HandleError, Table, TableRef};
use crate::memory::{Kind, MemoryObject};
use crate::process::{Authority, Objects, ObservationRef, Process, ProcessRef, Space};
use crate::queue::{self, Queue, QueueRef};
use crate::space::{self, Mapping, Target};
use crate::world::{self, World, registry};
use crate::{object, thread};

#[derive(Debug)]
struct ProcessConstruction {
    space: Option<space::Space>,
    authority: thread::SpaceAuthority,
}

impl ProcessConstruction {
    fn space(&mut self) -> Result<&mut space::Space, SpawnError> {
        self.space.as_mut().ok_or(SpawnError::Incomplete)
    }

    fn address(&mut self) -> Result<&mut AddressSpace, SpawnError> {
        Ok(self.space()?.address())
    }

    fn retain_authority(&mut self, authority: thread::SpaceAuthority) {
        self.authority = authority;
    }

    // A refused spawn's records are cleared and its tables reclaimed; a committed one left none.
    fn dismantle(&mut self, context: &mut thread::ThreadContext, world: &World) {
        let released = match core::mem::replace(&mut self.authority, thread::SpaceAuthority::None) {
            thread::SpaceAuthority::None => None,
            thread::SpaceAuthority::Released(released) => Some(released),
        };
        let Some(mut space) = self.space.take() else {
            return;
        };
        space.clear(context, world);
        if let Err(reason) = space.address().reclaim(released) {
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
    NotBindable(ActivateError),
    NoThread(thread::SpawnError),
    Incomplete,
    NoBootstrap(HandleError),
    NoTimePage(crate::timepage::Error),
    NoRing,
    NoWorld,
    NoMemory(Refused),
    NoAddress,
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
            Self::NoAddress => formatter.write_str("no free address for a kernel mapping"),
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
    let mut address = admitted_space()?;
    // The one world every process belongs to until worlds are objects.
    let Some((world_ref, world)) =
        world::root().and_then(|root| registry::hold(root).map(|world| (root, world)))
    else {
        if let Err(reason) = address.reclaim(None) {
            panic!("an unused space's table cleanup refused: {reason}");
        }
        return Err(SpawnError::NoWorld);
    };
    let mut construction = ProcessConstruction {
        space: None,
        authority: thread::SpaceAuthority::None,
    };
    let spawned = build(
        context,
        world_ref,
        &world,
        &mut construction,
        address,
        Named {
            name,
            which,
            argument,
            operand,
        },
    );
    construction.dismantle(context, &world);
    spawned
}

#[derive(Clone, Copy, Debug)]
struct Named {
    name: &'static str,
    which: u64,
    argument: u64,
    operand: u64,
}

fn build(
    context: &mut thread::ThreadContext,
    world_ref: NodeRef,
    world: &World,
    construction: &mut ProcessConstruction,
    address: AddressSpace,
    named: Named,
) -> Result<Spawned, SpawnError> {
    let Named {
        name,
        which,
        argument,
        operand,
    } = named;
    let clock = crate::timepage::root();
    let (mut earmark, layout) = lay_out(context, world_ref, world, construction, address)?;
    let Layout {
        entry,
        stack,
        ring,
        data,
        time,
    } = layout;
    clock
        .install(construction.address()?, time)
        .map_err(SpawnError::NoTimePage)?;
    let mut queue = Queue::new(clock, &mut earmark).ok_or(SpawnError::NoRing)?;
    queue
        .install(construction.address()?, ring, data)
        .map_err(SpawnError::NoSpace)?;
    let root = construction.address()?.root();

    let objects = Objects {
        ring: QueueRef::create(&mut earmark, world_ref, world, queue),
        table: TableRef::create(&mut earmark, world_ref, world),
    };
    // Created before the thread can observe the table it names.
    let bootstrap = object::bootstrap_identity(&mut earmark, world);
    let (_, table) = objects.table.hold().ok_or(SpawnError::NoWorld)?;
    let handle = match table.insert(context, bootstrap, Rights::UNIVERSAL) {
        Ok(handle) => handle,
        Err(reason) => {
            bootstrap.release(world);
            return Err(SpawnError::NoBootstrap(reason));
        }
    };
    drop(table);
    let binding = construction
        .address()?
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
        ring,
        data,
        time,
    };
    let observation = ObservationRef::create(&mut earmark, world_ref, world);
    let process = ProcessRef::create(
        &mut earmark,
        world_ref,
        world,
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

#[derive(Clone, Copy, Debug)]
struct Layout {
    entry: u64,
    stack: u64,
    ring: u64,
    data: u64,
    time: u64,
}

// Apart from `build`, so the record sets it works on are off the stack before the process is.
fn lay_out(
    context: &mut thread::ThreadContext,
    world_ref: NodeRef,
    world: &World,
    construction: &mut ProcessConstruction,
    address: AddressSpace,
) -> Result<(Earmark, Layout), SpawnError> {
    let prepared = load::plan()
        .map_err(SpawnError::Load)
        .and_then(|plan| Ok((earmark(context, world, &plan)?, plan)));
    let (mut earmark, plan) = match prepared {
        Ok(prepared) => prepared,
        Err(reason) => {
            let mut address = address;
            if let Err(cleanup) = address.reclaim(None) {
                panic!("an unused space's table cleanup refused: {cleanup}");
            }
            return Err(reason);
        }
    };
    let seed = world.placement_seed();
    construction.space = Some(space::Space::new(address, seed, &mut earmark));
    let space = construction.space()?;
    let stack = place_stack(context, world, &mut earmark, space)?;
    let mut place = |access| {
        let mapping = Mapping {
            access,
            target: Target::Kernel,
        };
        space
            .place(world, 1, 0, mapping)
            .ok_or(SpawnError::NoAddress)
    };
    let (ring, data, time) = (
        place(UserAccess::Write)?,
        place(UserAccess::Write)?,
        place(UserAccess::Read)?,
    );
    let entry = plan
        .load(context, world_ref, world, &mut earmark, space)
        .map_err(SpawnError::Load)?;
    Ok((
        earmark,
        Layout {
            entry,
            stack,
            ring,
            data,
            time,
        },
    ))
}

// Every arena frame the process's kernel objects can need, taken before anything is built.
fn earmark(
    context: &mut thread::ThreadContext,
    world: &World,
    plan: &Plan<'_>,
) -> Result<Earmark, SpawnError> {
    let records = world::ProcessPool::frames_for(1)
        .zip(world::ObservationPool::frames_for(1))
        .and_then(|(process, observation)| process.checked_add(observation));
    let stack = world::MemoryPool::frames_for(1)
        .zip(MemoryObject::frames_for(STACK_PAGES))
        .and_then(|(record, contents)| record.checked_add(contents));
    let frames = world::TablePool::frames_for(1)
        .zip(QueueRef::frames_for())
        .zip(world::ChannelPool::frames_for(1))
        .zip(records)
        .zip(stack.zip(plan.frames()))
        .and_then(|((((table, queue), channel), records), (stack, image))| {
            table
                .checked_add(Table::frames_for(world.handles()))?
                .checked_add(queue)?
                .checked_add(channel)?
                .checked_add(records)?
                .checked_add(stack)?
                .checked_add(image)?
                .checked_add(space::Space::frames_for())
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

const STACK_PAGES: u32 = (STACK_BYTES / PAGE_SIZE) as u32;

// A writable object one unmapped page above the next record below, so a run-off faults.
fn place_stack(
    context: &mut thread::ThreadContext,
    world: &World,
    earmark: &mut Earmark,
    space: &mut space::Space,
) -> Result<u64, SpawnError> {
    let object = MemoryObject::new(earmark, Kind::Sealable, STACK_PAGES);
    assert!(
        object.acquire_writer().is_ok(),
        "a fresh stack object refused its writer"
    );
    let memory = world.memories.insert(earmark, object);
    let mapping = Mapping {
        access: UserAccess::Write,
        target: Target::Object { memory, first: 0 },
    };
    let at = space
        .place(world, u64::from(STACK_PAGES), 1, mapping)
        .ok_or(SpawnError::NoAddress)?;
    space
        .fill(context, world, at)
        .map_err(SpawnError::NoSpace)?;
    Ok(at)
}
