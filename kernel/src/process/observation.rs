use core::sync::atomic::{AtomicU32, Ordering};

use world_tree::{NodeRef, Status};
use wost::mm::pool::Earmark;
use wost::sync::{Leaf, SpinLock};

use crate::arena::Local;
use crate::world::{World, registry};

// Where a process's status lands, in the spawner's world; the process holds only a weak name.
#[derive(Debug)]
pub(crate) struct Observation {
    status: SpinLock<Leaf, Option<Status>>,
    recorded: AtomicU32,
    #[cfg(feature = "selftest")]
    rites: SpinLock<Leaf, Option<Rites>>,
}

// What a teardown saw of a program as it released it, for the self-tests to compare.
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rites {
    pub(crate) handles: usize,
    pub(crate) served: usize,
    pub(crate) ending: Option<boot_record::Record>,
}

impl Observation {
    pub(crate) const fn new() -> Self {
        Self {
            status: SpinLock::new(None),
            recorded: AtomicU32::new(0),
            #[cfg(feature = "selftest")]
            rites: SpinLock::new(None),
        }
    }

    // Written once; a second status is refused and changes nothing.
    pub(crate) fn record(&self, status: Status) -> bool {
        let mut held = self.status.lock_leaf();
        if held.is_some() {
            return false;
        }
        *held = Some(status);
        self.recorded.fetch_add(1, Ordering::Release);
        true
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn status(&self) -> Option<Status> {
        *self.status.lock_leaf()
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn recorded(&self) -> u32 {
        self.recorded.load(Ordering::Acquire)
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn keep_rites(&self, rites: Rites) {
        *self.rites.lock_leaf() = Some(rites);
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn rites(&self) -> Option<Rites> {
        *self.rites.lock_leaf()
    }
}

// The spawner's strong reference; dropping it ends the observation once the process is done with it.
#[derive(Debug)]
pub(crate) struct ObservationRef {
    world: NodeRef,
    observation: Local<Observation>,
}

impl ObservationRef {
    pub(crate) fn create(earmark: &mut Earmark, world_ref: NodeRef, world: &World) -> Self {
        Self {
            world: world_ref,
            observation: world.observations.insert(earmark, Observation::new()),
        }
    }

    pub(crate) const fn local(&self) -> Local<Observation> {
        self.observation
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn status(&self) -> Option<Status> {
        let world = registry::hold(self.world)?;
        world.observations.hold(self.observation)?.status()
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn recorded(&self) -> u32 {
        registry::hold(self.world)
            .and_then(|world| world.observations.hold(self.observation))
            .map_or(0, |observation| observation.recorded())
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn rites(&self) -> Option<Rites> {
        let world = registry::hold(self.world)?;
        world.observations.hold(self.observation)?.rites()
    }
}

impl Drop for ObservationRef {
    fn drop(&mut self) {
        if let Some(world) = registry::hold(self.world) {
            let _ = world.observations.destroy(self.observation);
        }
    }
}
