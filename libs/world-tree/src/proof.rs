use vstd::prelude::verus;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
use crate::{
    BudgetRefusal, CreationEvent, CreationStep, DescendantRefusal, Limits, NodeRef, ProcessState,
    ProcessStep, Progress, WorldEvent, WorldState, WorldStep,
};
use crate::{Budgets, Creation, Descendants, Path};

verus! {

impl Path {
    // Tree: its path lists its ancestors and is at most 16 long.
    #[verifier::type_invariant]
    pub(crate) open spec fn path_at_most_16_long(self) -> bool {
        &&& self.depth <= 16
        &&& forall|at: int| self.depth <= at < 16 ==> self.nodes@[at] == crate::UNUSED
    }

    pub open(crate) spec fn ancestors(self) -> Seq<NodeRef> {
        self.nodes@.subrange(0, self.depth as int)
    }

    pub open(crate) spec fn at_distance(self, distance: int) -> Option<NodeRef> {
        if 0 <= distance < self.ancestors().len() {
            Some(self.ancestors()[self.ancestors().len() - 1 - distance])
        } else {
            None
        }
    }
}

pub proof fn lemma_distances_name_the_path(path: Path, world: NodeRef)
    ensures
        path.ancestors().contains(world) <==> exists|distance: int|
            path.at_distance(distance) == Some(world),
{
    let ancestors = path.ancestors();
    if ancestors.contains(world) {
        let at = choose|at: int| 0 <= at < ancestors.len() && ancestors[at] == world;
        assert(path.at_distance(ancestors.len() - 1 - at) == Some(world));
    }
    if exists|distance: int| path.at_distance(distance) == Some(world) {
        let distance = choose|distance: int| path.at_distance(distance) == Some(world);
        assert(ancestors[ancestors.len() - 1 - distance] == world);
    }
}

// Kill: each frame returns once, zeroed, to the nearest live ancestor.
pub open spec fn heir_is_nearest_live_ancestor(
    path: Path,
    states: Seq<WorldState>,
    distance: int,
) -> bool {
    &&& 0 <= distance < path.ancestors().len()
    &&& states[distance] == WorldState::Live
    &&& forall|nearer: int| 0 <= nearer < distance ==> states[nearer] != WorldState::Live
}

impl<const N: usize> Descendants<N> {
    #[verifier::type_invariant]
    pub(crate) open spec fn holds_a_set(self) -> bool {
        &&& self.count <= N
        &&& forall|i: int, j: int|
            0 <= i < j < self.count ==> self.entries@[i] != self.entries@[j]
    }

    pub open(crate) spec fn members(self) -> Seq<NodeRef> {
        self.entries@.subrange(0, self.count as int)
    }

    pub open(crate) spec fn tail_clear(self) -> bool {
        forall|at: int| self.count <= at < N ==> self.entries@[at] == crate::UNUSED
    }
}

// Tree: once a world is dying nothing joins its descendants.
pub open spec fn nothing_joins_once_dying(
    owner: WorldState,
    r: Result<(), DescendantRefusal>,
) -> bool {
    owner != WorldState::Live ==> r == Err(DescendantRefusal::Frozen)
}

// Tree: its descendant set holds exactly the worlds whose path contains it.
pub open spec fn descendant_sets_exact(
    paths: Map<NodeRef, Seq<NodeRef>>,
    sets: Map<NodeRef, Set<NodeRef>>,
) -> bool {
    &&& sets.dom() == paths.dom()
    &&& forall|world: NodeRef, other: NodeRef|
        #![trigger sets[world].contains(other)]
        #![trigger paths[other].contains(world)]
        paths.dom().contains(world) ==> (sets[world].contains(other) <==> (paths.dom().contains(
            other,
        ) && paths[other].contains(world)))
}

// The child enters the set of every ancestor on its path and starts with an empty set.
pub open spec fn entered_into(
    paths: Map<NodeRef, Seq<NodeRef>>,
    sets: Map<NodeRef, Set<NodeRef>>,
    child: NodeRef,
    path: Seq<NodeRef>,
) -> Map<NodeRef, Set<NodeRef>> {
    Map::new(
        paths.dom().insert(child),
        |world: NodeRef|
            if world == child {
                Set::empty()
            } else if path.contains(world) {
                sets[world].insert(child)
            } else {
                sets[world]
            },
    )
}

// The world leaves the set of every ancestor on its path, and its own set goes with it.
pub open spec fn detached_from(
    paths: Map<NodeRef, Seq<NodeRef>>,
    sets: Map<NodeRef, Set<NodeRef>>,
    world: NodeRef,
) -> Map<NodeRef, Set<NodeRef>> {
    Map::new(
        paths.dom().remove(world),
        |ancestor: NodeRef|
            if paths[world].contains(ancestor) {
                sets[ancestor].remove(world)
            } else {
                sets[ancestor]
            },
    )
}

// A world's reference carries a generation that is never issued twice, so a new child is on no
// path yet.
pub proof fn lemma_creation_keeps_sets_exact(
    paths: Map<NodeRef, Seq<NodeRef>>,
    sets: Map<NodeRef, Set<NodeRef>>,
    child: NodeRef,
    path: Seq<NodeRef>,
)
    requires
        descendant_sets_exact(paths, sets),
        !paths.dom().contains(child),
        !path.contains(child),
        forall|world: NodeRef| paths.dom().contains(world) ==> !paths[world].contains(child),
    ensures
        descendant_sets_exact(paths.insert(child, path), entered_into(paths, sets, child, path)),
{
    let after = paths.insert(child, path);
    let entered = entered_into(paths, sets, child, path);
    assert(entered.dom() =~= after.dom());
    assert forall|world: NodeRef, other: NodeRef| after.dom().contains(world) implies (
        entered[world].contains(other) <==> (after.dom().contains(other)
            && after[other].contains(world))
    ) by {
        if world != child {
            assert(sets[world].contains(other) <==> (paths.dom().contains(other)
                && paths[other].contains(world)));
        }
        if other != child && after.dom().contains(other) {
            assert(after[other] == paths[other]);
        }
    }
}

pub proof fn lemma_detach_keeps_sets_exact(
    paths: Map<NodeRef, Seq<NodeRef>>,
    sets: Map<NodeRef, Set<NodeRef>>,
    world: NodeRef,
)
    requires
        descendant_sets_exact(paths, sets),
        paths.dom().contains(world),
    ensures
        descendant_sets_exact(paths.remove(world), detached_from(paths, sets, world)),
{
    let after = paths.remove(world);
    let detached = detached_from(paths, sets, world);
    assert(detached.dom() =~= after.dom());
    assert forall|ancestor: NodeRef, other: NodeRef| after.dom().contains(ancestor) implies (
        detached[ancestor].contains(other) <==> (after.dom().contains(other)
            && after[other].contains(ancestor))
    ) by {
        assert(sets[ancestor].contains(other) <==> (paths.dom().contains(other)
            && paths[other].contains(ancestor)));
        if other != world && after.dom().contains(other) {
            assert(after[other] == paths[other]);
        }
    }
}

impl Creation {
    #[verifier::type_invariant]
    pub(crate) open spec fn below_its_ancestors(self) -> bool {
        0 < self.ancestors && names_an_ancestor(self.step, self.ancestors)
    }

    pub open(crate) spec fn spec_ancestors(self) -> u8 {
        self.ancestors
    }

    pub open(crate) spec fn spec_step(self) -> CreationStep {
        self.step
    }
}

pub open spec fn names_an_ancestor(step: CreationStep, ancestors: u8) -> bool {
    match step {
        CreationStep::Insert(distance)
        | CreationStep::Confirm(distance)
        | CreationStep::Remove(distance, _) => distance < ancestors,
        _ => true,
    }
}

// How many of the nearest ancestors hold the child in their sets at this step.
pub open spec fn entered(step: CreationStep, ancestors: u8) -> int {
    match step {
        CreationStep::Insert(distance) => distance as int,
        CreationStep::Remove(distance, _) => distance + 1,
        CreationStep::Publish
        | CreationStep::Confirm(_)
        | CreationStep::Published
        | CreationStep::Kill => ancestors as int,
        _ => 0,
    }
}

pub open spec fn ends(step: CreationStep) -> bool {
    step is Refused || step is Unwound || step is Published || step is Kill
}

// Tree: creating a world enters it into every ancestor's set or into none.
pub open spec fn enters_every_ancestor_or_none(
    before: Creation,
    event: CreationEvent,
    after: Creation,
) -> bool {
    let ancestors = before.spec_ancestors();
    let from = before.spec_step();
    let to = after.spec_step();
    &&& after.spec_ancestors() == ancestors
    &&& entered(to, ancestors) == match (from, event) {
        (CreationStep::Insert(_), CreationEvent::Done) => entered(from, ancestors) + 1,
        (CreationStep::Remove(..), CreationEvent::Done) => entered(from, ancestors) - 1,
        _ => entered(from, ancestors),
    }
    &&& ends(to) ==> entered(to, ancestors) == 0 || entered(to, ancestors) == ancestors
}

pub open spec fn debited(step: CreationStep) -> bool {
    !(step is Debit || step is Refused || step is Dissolve || step is Unwound)
}

// Budgets: debit and credit conserve the total, so an unwound creation returns what it took.
pub open spec fn credits_what_it_debited(
    before: Creation,
    event: CreationEvent,
    after: Creation,
) -> bool {
    let from = before.spec_step();
    debited(after.spec_step()) == match (from, event) {
        (CreationStep::Debit, CreationEvent::Done) => true,
        (CreationStep::Credit(_), CreationEvent::Done) => false,
        _ => debited(from),
    }
}

// Tree: every descendant is dying before the world is dead. A child published after an ancestor
// began dying was skipped by that ancestor's kill, so the creator confirms every ancestor first.
pub open spec fn published_after_every_confirm(
    before: Creation,
    event: CreationEvent,
    after: Creation,
) -> bool {
    let from = before.spec_step();
    match after.spec_step() {
        CreationStep::Confirm(0) => event == CreationEvent::Done && from == CreationStep::Publish,
        CreationStep::Confirm(distance) => event == CreationEvent::Done && from
            == CreationStep::Confirm((distance - 1) as u8),
        CreationStep::Published => event == CreationEvent::Done && from == CreationStep::Confirm(
            (before.spec_ancestors() - 1) as u8,
        ),
        CreationStep::Kill => from is Confirm && event is Halted,
        _ => true,
    }
}

// Tree: a world is live, dying or dead.
pub open spec fn live_then_dying_then_dead(
    state: WorldState,
    event: WorldEvent,
) -> Option<WorldState> {
    match (state, event) {
        (WorldState::Free, WorldEvent::Publish) => Some(WorldState::Live),
        (WorldState::Live, WorldEvent::Kill) => Some(WorldState::Dying),
        (WorldState::Dying, WorldEvent::Close) => Some(WorldState::Closed),
        _ => None,
    }
}

pub open spec fn world_steps_left(step: WorldStep) -> nat {
    match step {
        WorldStep::Kill => 11,
        WorldStep::ExitProcesses => 10,
        WorldStep::Quiesce => 9,
        WorldStep::AbandonQueuesAndTables => 8,
        WorldStep::DestroyObjects => 7,
        WorldStep::ClearSpaces => 6,
        WorldStep::RevokeMappers => 5,
        WorldStep::ClosePins => 4,
        WorldStep::Detach => 3,
        WorldStep::Dissolve => 2,
        WorldStep::Reclaim => 1,
        WorldStep::Finished => 0,
    }
}

pub open spec fn process_steps_left(step: ProcessStep) -> nat {
    match step {
        ProcessStep::Exit => 6,
        ProcessStep::Quiesce => 5,
        ProcessStep::AbandonQueue => 4,
        ProcessStep::DrainTable => 3,
        ProcessStep::ClearSpace => 2,
        ProcessStep::ReleaseRecords => 1,
        ProcessStep::Released => 0,
    }
}

// Kill: no step can refuse, and each finished step brings the teardown one step nearer its end.
pub open spec fn each_step_moves_toward_the_end(
    before: nat,
    after: nat,
    progress: Progress,
) -> bool {
    match progress {
        Progress::Pending => after == before,
        Progress::Done => after == if before == 0 {
            0
        } else {
            (before - 1) as nat
        },
    }
}

// Life: a process is running, then exited with one status, then released.
pub open spec fn running_then_exited_then_released(
    before: ProcessState,
    after: ProcessState,
) -> bool {
    ||| before is Running && after is Exited
    ||| before is Exited && after is Released
}

impl Budgets {
    // What is left never exceeds the limit.
    #[verifier::type_invariant]
    pub(crate) open spec fn within_limits(self) -> bool {
        self.descendants <= self.limits.descendants && self.threads <= self.limits.threads
    }

    pub open(crate) spec fn spec_limits(self) -> Limits {
        self.limits
    }

    pub open(crate) spec fn spec_descendants(self) -> u32 {
        self.descendants
    }

    pub open(crate) spec fn spec_threads(self) -> u32 {
        self.threads
    }
}

// Budgets: a debit moves exactly its amounts out of what is left.
pub open spec fn debit_conserves_the_total(
    before: Budgets,
    descendants: int,
    threads: int,
    r: Result<Budgets, BudgetRefusal>,
) -> bool {
    match r {
        Ok(after) => {
            &&& after.spec_limits() == before.spec_limits()
            &&& after.spec_descendants() + descendants == before.spec_descendants()
            &&& after.spec_threads() + threads == before.spec_threads()
        },
        Err(_) => true,
    }
}

// Budgets: a credit moves exactly its amounts back into what is left.
pub open spec fn credit_conserves_the_total(
    before: Budgets,
    descendants: int,
    threads: int,
    r: Result<Budgets, BudgetRefusal>,
) -> bool {
    match r {
        Ok(after) => {
            &&& after.spec_limits() == before.spec_limits()
            &&& after.spec_descendants() == before.spec_descendants() + descendants
            &&& after.spec_threads() == before.spec_threads() + threads
        },
        Err(_) => true,
    }
}

} // verus!
