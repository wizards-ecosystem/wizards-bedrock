mod observation;
mod teardown;

use boot_record::Record;
use world_tree::{NodeRef, ProcessState, ProcessStep, Status};
use wost::arch::current::task;
use wost::mm::pool::Earmark;
use wost::mm::slab::{Hold, Refusal, Removal};
use wost::sync::{Leaf, SpinLock};

use crate::arena::Local;
use crate::handle::TableRef;
use crate::queue::QueueRef;
use crate::user::Launch;
use crate::world::{World, registry};

#[cfg(feature = "selftest")]
pub(crate) use observation::Rites;
pub(crate) use observation::{Observation, ObservationRef};
pub(crate) use teardown::step;

// A process's table and queue in its world; each goes when its last reference does.
#[derive(Debug)]
pub(crate) struct Objects {
    pub(crate) table: TableRef,
    pub(crate) ring: QueueRef,
}

impl Objects {
    // Idempotent, so a teardown refused by a passing hold retries it whole.
    #[must_use = "a refused destroy leaves an object for a retry"]
    fn destroy(&self) -> bool {
        let ring = self.ring.destroy();
        let table = self.table.destroy();
        ring && table
    }
}

impl Drop for Objects {
    fn drop(&mut self) {
        let destroyed = self.destroy();
        assert!(
            destroyed,
            "a process's objects outlived their last reference"
        );
    }
}

#[derive(Debug)]
pub(crate) enum Authority {
    Bound(task::Bound),
    Released(task::Released),
}

// The address space and everything mapped in it, owned by the process until its teardown clears it.
#[derive(Debug)]
pub(crate) struct Space {
    pub(crate) space: crate::space::Space,
    pub(crate) authority: Option<Authority>,
}

#[derive(Debug)]
struct Life {
    state: ProcessState,
    step: ProcessStep,
    // Set by the thread just before it retires; the teardown turns it into the status.
    ending: Option<Record>,
    slot: Option<usize>,
    // Taken while the table and ring still live; observable only once the record is gone.
    #[cfg(feature = "selftest")]
    rites: Option<Rites>,
}

#[derive(Debug)]
pub(crate) struct Process {
    launch: Launch,
    observation: Local<Observation>,
    objects: Objects,
    life: SpinLock<Leaf, Life>,
    space: SpinLock<Leaf, Option<Space>>,
}

impl Process {
    fn new(launch: Launch, observation: Local<Observation>, objects: Objects) -> Self {
        Self {
            launch,
            observation,
            objects,
            life: SpinLock::new(Life {
                state: ProcessState::Running,
                step: ProcessStep::Exit,
                ending: None,
                slot: None,
                #[cfg(feature = "selftest")]
                rites: None,
            }),
            space: SpinLock::new(None),
        }
    }

    pub(crate) const fn launch(&self) -> Launch {
        self.launch
    }

    pub(crate) const fn objects(&self) -> &Objects {
        &self.objects
    }

    // The thread is published and its space bound: from here the teardown owns both.
    pub(crate) fn commit(&self, slot: usize, space: Space) {
        self.life.lock_leaf().slot = Some(slot);
        let mut held = self.space.lock_leaf();
        assert!(held.is_none(), "a process was given a second address space");
        *held = Some(space);
    }

    // The space leaves the process for one operation, so no lock is held while it maps or waits;
    // the caller is the process's one thread, and the teardown takes the space only after it.
    pub(crate) fn with_space<R>(
        &self,
        operate: impl FnOnce(&mut crate::space::Space) -> R,
    ) -> Option<R> {
        let mut space = self.space.lock_leaf().take()?;
        let result = operate(&mut space.space);
        let mut held = self.space.lock_leaf();
        assert!(held.is_none(), "a process was given a second address space");
        *held = Some(space);
        Some(result)
    }

    pub(crate) fn record_ending(&self, ending: Record) {
        let mut life = self.life.lock_leaf();
        if life.ending.is_none() {
            life.ending = Some(ending);
        }
    }
}

// Names a process by its world and its place in that world's arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ProcessRef {
    world: NodeRef,
    process: Local<Process>,
}

impl ProcessRef {
    pub(crate) fn create(
        earmark: &mut Earmark,
        world_ref: NodeRef,
        world: &World,
        launch: Launch,
        observation: Local<Observation>,
        objects: Objects,
    ) -> Self {
        let process = world
            .processes
            .insert(earmark, || Process::new(launch, observation, objects));
        Self {
            world: world_ref,
            process,
        }
    }

    pub(crate) const fn world(self) -> NodeRef {
        self.world
    }

    pub(crate) fn hold(self) -> Option<(Hold<World>, Hold<Process>)> {
        let world = registry::hold(self.world)?;
        let process = world.processes.hold(self.process)?;
        Some((world, process))
    }

    // `false` while a hold still borrows the record; a record already gone counts as destroyed.
    pub(crate) fn destroy(self) -> bool {
        registry::hold(self.world).is_none_or(|world| {
            matches!(
                world.processes.destroy(self.process),
                Ok(_) | Err(Removal::Refused(Refusal::Stale))
            )
        })
    }
}

// The status an ending records: what the program asked for, or what stopped it.
fn status_of(ending: Option<Record>) -> Status {
    match ending {
        Some(Record::Exit(exit)) => Status::Exited(exit.status),
        Some(Record::Fault(fault)) => {
            Status::Faulted(u8::try_from(fault.vector).unwrap_or(u8::MAX))
        }
        _ => Status::Killed,
    }
}
