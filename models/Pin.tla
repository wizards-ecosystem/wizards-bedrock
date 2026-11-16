--------------------------------- MODULE Pin ---------------------------------
\* The counted pin on a world node: a weak reference names the node and its generation, the
\* node's registry word holds the pin count and the world state in one atomic, and a pin loads
\* the generation, compare-exchanges the word from an admitted state to count + 1, then reloads
\* the generation. Teardown closes the word, waits for the count to drain, reuses the frames,
\* then advances the generation so the slot can hold a new world.
EXTENDS Naturals, FiniteSets

CONSTANTS
    Generations,    \* Worlds the node slot holds in turn.
    Revalidate      \* A pin rereads the generation after counting; FALSE is the mutation.

ASSUME Generations \in Nat \ {0}
ASSUME Revalidate \in BOOLEAN

\* An ordinary pin admits only a Live world; a teardown helper also admits a Dying one.
Pinners == {"ordinary", "helper"}
Admits(p) == IF p = "ordinary" THEN {"Live"} ELSE {"Live", "Dying"}

VARIABLES
    generation, \* The node's generation.
    worldState, \* The state half of the registry word.
    count,      \* The pin count half of the registry word.
    reused,     \* Generations whose frames were zeroed and reused.
    atClose,    \* The pins counted when the word was closed.
    pc,         \* Each pinner's program counter.
    expected    \* The generation each pinner's weak reference names.

vars == << generation, worldState, count, reused, atClose, pc, expected >>

Init ==
    /\ generation = 0
    /\ worldState = "Live"
    /\ count = 0
    /\ reused = {}
    /\ atClose = {}
    /\ pc = [p \in Pinners |-> "idle"]
    /\ expected = [p \in Pinners |-> 0]

Counted == {p \in Pinners: pc[p] \in {"counted", "pinned"}}

\* A reference to an older generation fails this first check, so only the current one proceeds.
Load(p) ==
    /\ pc[p] = "idle"
    /\ expected' = [expected EXCEPT ![p] = generation]
    /\ pc' = [pc EXCEPT ![p] = "loaded"]
    /\ UNCHANGED << generation, worldState, count, reused, atClose >>

Count(p) ==
    /\ pc[p] = "loaded"
    /\ IF worldState \in Admits(p)
       THEN /\ count' = count + 1
            /\ pc' = [pc EXCEPT ![p] = "counted"]
       ELSE /\ pc' = [pc EXCEPT ![p] = "idle"]
            /\ UNCHANGED count
    /\ UNCHANGED << generation, worldState, reused, atClose, expected >>

Reload(p) ==
    /\ pc[p] = "counted"
    /\ IF ~Revalidate \/ generation = expected[p]
       THEN /\ pc' = [pc EXCEPT ![p] = "pinned"]
            /\ UNCHANGED count
       ELSE /\ pc' = [pc EXCEPT ![p] = "idle"]
            /\ count' = count - 1
    /\ UNCHANGED << generation, worldState, reused, atClose, expected >>

Unpin(p) ==
    /\ pc[p] = "pinned"
    /\ pc' = [pc EXCEPT ![p] = "idle"]
    /\ count' = count - 1
    /\ UNCHANGED << generation, worldState, reused, atClose, expected >>

Kill ==
    /\ worldState = "Live"
    /\ worldState' = "Dying"
    /\ UNCHANGED << generation, count, reused, atClose, pc, expected >>

Close ==
    /\ worldState = "Dying"
    /\ worldState' = "Closed"
    /\ atClose' = Counted
    /\ UNCHANGED << generation, count, reused, pc, expected >>

\* Once the count drains, the world's frames are zeroed and given to the heir.
Dissolve ==
    /\ worldState = "Closed"
    /\ count = 0
    /\ generation \notin reused
    /\ reused' = reused \cup {generation}
    /\ UNCHANGED << generation, worldState, count, atClose, pc, expected >>

Retire ==
    /\ worldState = "Closed"
    /\ generation \in reused
    /\ generation + 1 < Generations
    /\ generation' = generation + 1
    /\ worldState' = "Free"
    /\ atClose' = {}
    /\ UNCHANGED << count, reused, pc, expected >>

Create ==
    /\ worldState = "Free"
    /\ worldState' = "Live"
    /\ UNCHANGED << generation, count, reused, atClose, pc, expected >>

Next ==
    \/ \E p \in Pinners: Load(p) \/ Count(p) \/ Reload(p) \/ Unpin(p)
    \/ Kill \/ Close \/ Dissolve \/ Retire \/ Create

Spec == Init /\ [][Next]_vars

\* Grace period and Containment: a world's frames are not reused while a pin on it is held, so a
\* weak reference never reaches a later world through a reused slot.
NoReuseWhilePinned == \A p \in Pinners: pc[p] = "pinned" => expected[p] \notin reused

\* Kill: a dying world's teardown never waits on a pin taken after it closed.
NoPinAfterClose == worldState = "Closed" => Counted \subseteq atClose

CountIsPins == count = Cardinality(Counted)

TypeOK ==
    /\ generation \in 0..(Generations - 1)
    /\ worldState \in {"Free", "Live", "Dying", "Closed"}
    /\ count \in 0..Cardinality(Pinners)
    /\ reused \subseteq 0..(Generations - 1)
    /\ atClose \subseteq Pinners
    /\ pc \in [Pinners -> {"idle", "loaded", "counted", "pinned"}]
    /\ expected \in [Pinners -> 0..(Generations - 1)]

=============================================================================
