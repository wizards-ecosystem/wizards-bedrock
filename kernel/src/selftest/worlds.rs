use world_tree::{Admission, Limits, NodeRef};
use wost::mm::pool::{self, ArenaId, FrameKind};

use super::threads::yields_until;
use crate::thread::ThreadContext;
use crate::world::{self, Request, registry};

const PER_COLOUR: u32 = 8;

const LIMITS: Limits = Limits {
    descendants: 0,
    threads: 0,
    handles: 16,
};

const SAMPLES: u32 = 4;

fn child_of(context: &mut ThreadContext, parent: NodeRef, mask: u64) -> Option<NodeRef> {
    child_with(context, parent, mask, PER_COLOUR)
}

// A child of the whole parent's mask that can hold a few programs' handles.
pub(super) fn child(
    context: &mut ThreadContext,
    parent: NodeRef,
    per_colour: u32,
) -> Option<NodeRef> {
    child_with(context, parent, 0, per_colour)
}

fn child_with(
    context: &mut ThreadContext,
    parent: NodeRef,
    mask: u64,
    per_colour: u32,
) -> Option<NodeRef> {
    let request = Request {
        per_colour,
        mask,
        limits: LIMITS,
    };
    world::create(context, parent, request).ok()
}

pub(super) fn arena(node: NodeRef) -> Option<ArenaId> {
    registry::pin(node, Admission::Live).map(|world| world.arena())
}

// The kill is done once the reaper is idle and the parent holds the given reservation again.
pub(super) fn killed(
    context: &mut ThreadContext,
    node: NodeRef,
    parent: NodeRef,
    reserved: usize,
) -> bool {
    world::kill(node)
        && yields_until(context, || {
            crate::reaper::is_idle()
                && world::inspect(parent).is_some_and(|(held, _)| held == reserved)
        })
}

pub(super) fn a_world_past_its_reservation_is_refused_alone(context: &mut ThreadContext) -> bool {
    let Some(root) = world::root() else {
        return false;
    };
    let Some((initial, _)) = world::inspect(root) else {
        return false;
    };
    let (Some(greedy), Some(sibling)) = (child_of(context, root, 0), child_of(context, root, 0))
    else {
        return false;
    };
    let (Some((parent, parent_used)), Some((reserved, used)), Some(other), Some(arena)) = (
        world::inspect(root),
        world::inspect(greedy),
        world::inspect(sibling),
        arena(greedy),
    ) else {
        return false;
    };
    let refused = match pool::earmark(context, arena, (reserved - used) as u32) {
        Ok(_whole) => pool::earmark(context, arena, 1).err() == Some(pool::Refused::NoMemory),
        Err(_) => false,
    };
    let untouched = world::inspect(sibling) == Some(other);
    refused
        && untouched
        && killed(context, greedy, root, parent + reserved)
        && world::inspect(root).is_some_and(|(_, root_used)| root_used == parent_used)
        && killed(context, sibling, root, initial)
        && disjoint_colours_stay_apart(context, root, initial)
        && creates_after_a_kill_fit_every_set(context, root, initial)
}

// Children enough to fill the root's set beside their parent and grandparent.
const FANOUT: u32 = 61;

fn tree_child(
    context: &mut ThreadContext,
    parent: NodeRef,
    per_colour: u32,
    descendants: u32,
) -> Option<NodeRef> {
    let request = Request {
        per_colour,
        mask: 1,
        limits: Limits {
            descendants,
            ..LIMITS
        },
    };
    world::create(context, parent, request).ok()
}

// A killed world gives back only its own share while its descendants still die, so creating in
// its parent at once fits every ancestor's set or is refused, and never overflows the root's.
fn creates_after_a_kill_fit_every_set(
    context: &mut ThreadContext,
    root: NodeRef,
    initial: usize,
) -> bool {
    let Some(top) = tree_child(context, root, PER_COLOUR * (FANOUT + 4), FANOUT + 1) else {
        return false;
    };
    let middle = tree_child(context, top, PER_COLOUR * (FANOUT + 1), FANOUT);
    let filled = middle.is_some_and(|middle| {
        (0..FANOUT).all(|_| tree_child(context, middle, PER_COLOUR, 0).is_some())
    });
    let detached = filled
        && middle.is_some_and(|middle| {
            world::kill(middle) && yields_until(context, || registry::state(middle).is_none())
        });
    if detached {
        for _ in 0..2 {
            let _ = tree_child(context, top, PER_COLOUR, 0);
        }
    }
    killed(context, top, root, initial) && detached
}

// Each sibling's frames carry only the colours of its own mask.
fn disjoint_colours_stay_apart(context: &mut ThreadContext, root: NodeRef, initial: usize) -> bool {
    let colours = pool::colours();
    if colours < 2 {
        return true;
    }
    let even = (0..colours)
        .step_by(2)
        .fold(0u64, |mask, colour| mask | 1u64 << colour);
    let all = u64::MAX >> (u64::BITS - colours);
    let odd = all & !even;
    let (Some(first), Some(second)) = (child_of(context, root, even), child_of(context, root, odd))
    else {
        return false;
    };
    let apart = [(first, even), (second, odd)]
        .into_iter()
        .all(|(node, mask)| arena(node).is_some_and(|arena| frames_within(context, arena, mask)));
    let first_gone = killed(
        context,
        first,
        root,
        initial - (PER_COLOUR * odd.count_ones()) as usize,
    );
    apart && first_gone && killed(context, second, root, initial)
}

fn frames_within(context: &mut ThreadContext, arena: ArenaId, mask: u64) -> bool {
    let Ok(mut earmark) = pool::earmark(context, arena, SAMPLES) else {
        return false;
    };
    (0..SAMPLES).all(|_| {
        earmark.take(FrameKind::ArenaData).is_some_and(|frame| {
            let colour = (frame.address().get() >> 12) % u64::from(pool::colours());
            mask & 1 << colour != 0
        })
    })
}
