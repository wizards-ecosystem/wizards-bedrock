use core::fmt;

use world_tree::NodeRef;
use wost::arch::current::paging::{ActivateError, AddressSpace, MapError, UserAccess};
use wost::arch::current::{cpu, xstate};
use wost::mm::pool::{self as frames, Earmark, Refused};
use x86_paging::PAGE_SIZE;

use super::call::cross;
use super::load::{self, LoadError, Plan};
use super::{Launch, STACK_BYTES, Spawned};
use crate::channel::{self, Endpoint, Peer};
use crate::handle::{HandleError, Table, TableRef};
use crate::memory::{Kind, MemoryObject};
use crate::object::ObjectRef;
use crate::process::{Authority, Objects, ObservationRef, ProcessRef, Space};
use crate::queue::{self, QueueRef, SharedQueue};
use crate::space::{self, Mapping, Page, Target};
use crate::thread;
use crate::world::{self, World, registry};

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
    match world::root() {
        Some(root) => spawn_in(context, root, None, name, which, argument, operand),
        None => admitted_isolation().and(Err(SpawnError::NoWorld)),
    }
}

// A program in the given world, its objects charged to that world's arena; `birth` is an endpoint
// the caller holds a strong reference to, consumed whether or not the spawn succeeds.
pub(crate) fn spawn_in(
    context: &mut thread::ThreadContext,
    world_ref: NodeRef,
    mut birth: Option<Peer>,
    name: &'static str,
    which: u64,
    argument: u64,
    operand: u64,
) -> Result<Spawned, SpawnError> {
    let spawned = spawn_with(
        context, world_ref, &mut birth, name, which, argument, operand,
    );
    if let Some(unplaced) = birth {
        channel::forfeit(unplaced);
    }
    spawned
}

fn spawn_with(
    context: &mut thread::ThreadContext,
    world_ref: NodeRef,
    birth: &mut Option<Peer>,
    name: &'static str,
    which: u64,
    argument: u64,
    operand: u64,
) -> Result<Spawned, SpawnError> {
    let isolated = admitted_isolation()?;
    let world = registry::hold(world_ref).ok_or(SpawnError::NoWorld)?;
    let mut construction = ProcessConstruction {
        space: None,
        authority: thread::SpaceAuthority::None,
    };
    // Siblings, not nested: the placement and record work in `lay_out` never runs under `build`.
    let spawned = lay_out(context, world_ref, &world, &mut construction, isolated).and_then(
        |(earmark, layout)| {
            build(
                context,
                world_ref,
                &world,
                &mut construction,
                (earmark, layout),
                birth,
                Named {
                    name,
                    which,
                    argument,
                    operand,
                },
            )
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
    (mut earmark, layout): (Earmark, Layout),
    birth: &mut Option<Peer>,
    named: Named,
) -> Result<Spawned, SpawnError> {
    let Named {
        name,
        which,
        argument,
        operand,
    } = named;
    let clock = crate::timepage::root();
    let Layout {
        entry,
        stack,
        ring,
        data,
        time,
    } = layout;
    let time_page = clock
        .frame()
        .ok_or(SpawnError::NoTimePage(crate::timepage::Error::Absent))?;
    let shared = SharedQueue::new(&mut earmark).ok_or(SpawnError::NoRing)?;
    let (ring_page, data_page) = shared.frames();
    for (at, frame) in [(time, time_page), (ring, ring_page), (data, data_page)] {
        construction
            .space()?
            .share(&mut earmark, at, frame)
            .map_err(SpawnError::NoSpace)?;
    }
    let root = construction.address()?.root();

    let objects = Objects {
        ring: QueueRef::create(&mut earmark, world_ref, world, clock, shared),
        table: TableRef::create(&mut earmark, world_ref, world),
    };
    // Created before the thread can observe the table it names.
    let placed = match birth.take() {
        None => channel::alone(&mut earmark, world),
        Some(given) if given.world == world_ref => Some(given.endpoint),
        Some(given) => channel::rehome(&mut earmark, world, given),
    };
    let Some(endpoint) = placed else {
        return Err(SpawnError::NoBootstrap(HandleError::Gone));
    };
    let bootstrap = ObjectRef::Endpoint(endpoint);
    let (_, table) = objects.table.hold().ok_or(SpawnError::NoWorld)?;
    let handle = match table.insert(context, bootstrap, channel::RIGHTS) {
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
        launch,
        observation.local(),
        objects,
    );
    // The process takes the space only once its thread is bound to it.
    let arrange = |thread, bound| {
        let Some(space) = construction.space.take() else {
            return Err(bound);
        };
        let Some((_, held)) = process.hold() else {
            construction.space = Some(space);
            return Err(bound);
        };
        held.commit(
            thread,
            Space {
                space,
                authority: Some(Authority::Bound(bound)),
            },
        );
        Ok(())
    };
    let spawned = thread::spawn_in_space(context, name, cross, binding, clock, process, arrange);
    let thread = match spawned {
        Ok(thread) => thread.id(),
        Err(failure) => {
            construction.retain_authority(failure.authority);
            assert!(process.destroy(), "a process that never ran was borrowed");
            return Err(SpawnError::NoThread(failure.reason));
        }
    };

    Ok(Spawned {
        thread,
        observation,
        birth: endpoint,
    })
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    entry: u64,
    stack: u64,
    ring: u64,
    data: u64,
    time: u64,
}

fn lay_out(
    context: &mut thread::ThreadContext,
    world_ref: NodeRef,
    world: &World,
    construction: &mut ProcessConstruction,
    isolated: bool,
) -> Result<(Earmark, Layout), SpawnError> {
    let plan = load::plan().map_err(SpawnError::Load)?;
    let mut earmark = earmark(context, world, &plan, isolated)?;
    let address = if isolated {
        AddressSpace::for_user_isolated(&mut earmark)
    } else {
        AddressSpace::for_user(&mut earmark)
    }
    .map_err(SpawnError::NoSpace)?;
    let seed = world.placement_seed();
    construction.space = Some(space::Space::new(address, seed, &mut earmark));
    let space = construction.space()?;
    let stack = place_stack(context, world, &mut earmark, space)?;
    let mut place = |access, page| {
        let mapping = Mapping {
            access,
            target: Target::Shared(page),
        };
        space
            .place(world, 1, 0, mapping)
            .ok_or(SpawnError::NoAddress)
    };
    let (ring, data, time) = (
        place(UserAccess::Write, Page::Ring)?,
        place(UserAccess::Write, Page::Data)?,
        place(UserAccess::Read, Page::Time)?,
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
    isolated: bool,
) -> Result<Earmark, SpawnError> {
    let records = world::ProcessPool::frames_for(1)
        .zip(world::ObservationPool::frames_for(1))
        .and_then(|(process, observation)| process.checked_add(observation));
    let stack_tables = u32::try_from(x86_paging::tables_for(u64::from(STACK_PAGES))).ok();
    let shared_tables = u32::try_from(3 * x86_paging::tables_for(1)).ok();
    let stack = world::MemoryPool::frames_for(1)
        .zip(MemoryObject::frames_for(STACK_PAGES))
        .zip(stack_tables)
        .and_then(|((record, contents), tables)| record.checked_add(contents)?.checked_add(tables));
    let frames = world::TablePool::frames_for(1)
        .zip(QueueRef::frames_for())
        .zip(Endpoint::rehome_frames_for(1))
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
                .checked_add(space::Space::frames_for())?
                .checked_add(AddressSpace::user_root_tables(isolated)?)?
                .checked_add(shared_tables?)
        })
        .ok_or(SpawnError::NoMemory(Refused::NoMemory))?;
    frames::earmark(context, world.arena(), frames).map_err(SpawnError::NoMemory)
}

fn admitted_isolation() -> Result<bool, SpawnError> {
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
        crate::interrupts::UserIsolation::Closed => Err(SpawnError::NoUserIsolation),
        crate::interrupts::UserIsolation::Shared => Ok(false),
        crate::interrupts::UserIsolation::Paired => Ok(true),
    }
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
    let memory = world.memories.insert(earmark, || object);
    let mapping = Mapping {
        access: UserAccess::Write,
        target: Target::Object { memory, first: 0 },
    };
    let at = space
        .place(world, u64::from(STACK_PAGES), 1, mapping)
        .ok_or(SpawnError::NoAddress)?;
    space
        .fill(context, world, earmark, at)
        .map_err(SpawnError::NoSpace)?;
    Ok(at)
}
