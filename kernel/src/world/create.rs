use world_tree::{
    Admission, Creation, CreationEvent, CreationRefusal, CreationStep, Limits, NodeRef, Path,
};
use wost::mm::pool::{self, ArenaId, ColourMask, Refused};

use super::{World, WorldKey, registry, teardown};
use crate::random;
use crate::thread::ThreadContext;

// What a parent gives a child: frames of each colour in the mask (0 for the parent's) and limits.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Request {
    pub(crate) per_colour: u32,
    pub(crate) mask: u64,
    pub(crate) limits: Limits,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Refusal {
    NoMemory,
    Limit,
    Dead,
}

impl From<CreationRefusal> for Refusal {
    fn from(refusal: CreationRefusal) -> Self {
        match refusal {
            CreationRefusal::Budget => Self::Limit,
            CreationRefusal::NoMemory => Self::NoMemory,
            CreationRefusal::Dead => Self::Dead,
        }
    }
}

struct Child {
    parent: NodeRef,
    path: Path,
    mask: ColourMask,
    request: Request,
    arena: Option<ArenaId>,
    node: Option<NodeRef>,
}

// The child enters every ancestor's descendants or none; one that died meanwhile kills it.
pub(crate) fn create(
    context: &mut ThreadContext,
    parent: NodeRef,
    request: Request,
) -> Result<NodeRef, Refusal> {
    let (path, mask) = {
        let world = registry::pin(parent, Admission::Live).ok_or(Refusal::Dead)?;
        let path = world.path.child(parent).map_err(|_| Refusal::Limit)?;
        let mask = match request.mask {
            0 => world.mask,
            bits => pool::colour_mask(bits).ok_or(Refusal::Limit)?,
        };
        (path, mask)
    };
    let Some(mut creation) = Creation::new(&path) else {
        panic!("a child path is never the root path");
    };
    let mut child = Child {
        parent,
        path,
        mask,
        request,
        arena: None,
        node: None,
    };
    loop {
        let event = match creation.step() {
            CreationStep::Debit => child.debit(context),
            CreationStep::Build => child.build(context),
            CreationStep::Insert(distance) => child.insert(distance),
            CreationStep::Publish => {
                registry::publish(child.node());
                CreationEvent::Done
            }
            CreationStep::Confirm(distance) => child.confirm(distance),
            CreationStep::Remove(distance, _) => child.remove(distance),
            CreationStep::Credit(_) => child.credit(),
            CreationStep::Dissolve(_) => child.dissolve(),
            CreationStep::Refused(cause) | CreationStep::Unwound(cause) => {
                return Err(cause.into());
            }
            CreationStep::Published => return Ok(child.node()),
            CreationStep::Kill => {
                teardown::kill(child.node());
                return Err(Refusal::Dead);
            }
        };
        let Ok(next) = creation.next(event) else {
            panic!("world creation took an event its step does not admit");
        };
        creation = next;
    }
}

impl Child {
    fn node(&self) -> NodeRef {
        self.node
            .unwrap_or_else(|| panic!("a world creation used its node before building it"))
    }

    fn ancestor(&self, distance: u8) -> NodeRef {
        self.path
            .nearest(distance)
            .unwrap_or_else(|| panic!("a world creation reached past its path"))
    }

    // The budgets go back if the donation is refused, so a refusal leaves the parent as it was.
    fn debit(&mut self, context: &mut ThreadContext) -> CreationEvent {
        let Some(parent) = registry::pin(self.parent, Admission::Live) else {
            return CreationEvent::Halted(CreationRefusal::Dead);
        };
        {
            let mut life = parent.life.lock_leaf();
            match life.budgets.debit_child(self.request.limits) {
                Ok(budgets) => life.budgets = budgets,
                Err(_) => return CreationEvent::Halted(CreationRefusal::Budget),
            }
        }
        match pool::donate(context, parent.arena, self.mask, self.request.per_colour) {
            Ok(arena) => {
                self.arena = Some(arena);
                CreationEvent::Done
            }
            Err(refused) => {
                credit(&parent, self.request.limits);
                CreationEvent::Halted(match refused {
                    Refused::NoMemory | Refused::NoNode => CreationRefusal::NoMemory,
                    Refused::Mask => CreationRefusal::Budget,
                    Refused::Stale | Refused::Busy | Refused::Twice => CreationRefusal::Dead,
                })
            }
        }
    }

    fn build(&mut self, context: &mut ThreadContext) -> CreationEvent {
        let Some(arena) = self.arena else {
            panic!("a world was built before its arena was donated");
        };
        let mut key = WorldKey([0; 32]);
        if let Err(reason) = random::fill(&mut key.0) {
            panic!("a world was created before the generator was ready: {reason}");
        }
        let world = World::new(arena, &key, self.path, self.mask, self.request.limits);
        match registry::prepare(context, arena, world) {
            Ok(node) => {
                self.node = Some(node);
                CreationEvent::Done
            }
            Err(_) => CreationEvent::Halted(CreationRefusal::NoMemory),
        }
    }

    fn insert(&self, distance: u8) -> CreationEvent {
        let Some(ancestor) = registry::pin(self.ancestor(distance), Admission::Teardown) else {
            return CreationEvent::Halted(CreationRefusal::Dead);
        };
        let mut life = ancestor.life.lock_leaf();
        match life.descendants.insert(ancestor.state(), self.node()) {
            Ok(()) => CreationEvent::Done,
            Err(world_tree::DescendantRefusal::Frozen) => {
                CreationEvent::Halted(CreationRefusal::Dead)
            }
            Err(refusal) => panic!("a budgeted child could not join its ancestor: {refusal}"),
        }
    }

    // Read after the publish: an ancestor dying since its insert skipped this unpublished child.
    fn confirm(&self, distance: u8) -> CreationEvent {
        let live = registry::pin(self.ancestor(distance), Admission::Teardown)
            .is_some_and(|ancestor| ancestor.state() == world_tree::WorldState::Live);
        if live {
            CreationEvent::Done
        } else {
            CreationEvent::Halted(CreationRefusal::Dead)
        }
    }

    fn remove(&self, distance: u8) -> CreationEvent {
        if let Some(ancestor) = registry::pin(self.ancestor(distance), Admission::Teardown) {
            let removed = ancestor.life.lock_leaf().descendants.remove(self.node());
            assert!(
                removed.is_ok(),
                "an unwinding child was missing from its ancestor"
            );
        }
        CreationEvent::Done
    }

    fn credit(&self) -> CreationEvent {
        if let Some(parent) = registry::pin(self.parent, Admission::Teardown) {
            credit(&parent, self.request.limits);
        }
        CreationEvent::Done
    }

    fn dissolve(&mut self) -> CreationEvent {
        if let Some(node) = self.node.take() {
            registry::withdraw(node).release();
        }
        let Some(arena) = self.arena else {
            panic!("a world creation dissolved an arena it never had");
        };
        match teardown::dissolve(&self.path, arena, teardown::DISSOLVE_CHUNK) {
            world_tree::Progress::Done => CreationEvent::Done,
            world_tree::Progress::Pending => CreationEvent::Pending,
        }
    }
}

pub(super) fn credit(parent: &World, limits: Limits) {
    let mut life = parent.life.lock_leaf();
    match life.budgets.credit_child(limits) {
        Ok(budgets) => life.budgets = budgets,
        Err(refusal) => panic!("a parent was credited a child it never debited: {refusal}"),
    }
}
