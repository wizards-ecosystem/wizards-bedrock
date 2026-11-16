------------------------------ MODULE Shootdown ------------------------------
\* TLB shootdown for one address space as `libs/tlb-generation` computes it: a per-space
\* generation, a CPU set a CPU joins before its root load and leaves when its PCID slot is
\* evicted, one PCID slot per CPU holding the generation it has seen, and acknowledgement
\* accounting per request. CPU p unmaps page p; every CPU may run the space and cache pages.
\* Interrupts stay on while a requester waits, so its own handler keeps answering others.
EXTENDS Naturals, FiniteSets

CONSTANTS
    CpuCount,           \* CPUs that may run the space; the first `PageCount` also unmap.
    PageCount,          \* Pages, each unmapped once by the CPU of the same number.
    StaleSlotFlushes    \* A load over a stale slot flushes; FALSE is the mutation.

ASSUME CpuCount \in Nat \ {0}
ASSUME PageCount \in 1..CpuCount
ASSUME StaleSlotFlushes \in BOOLEAN

Cpus == 1..CpuCount
Pages == 1..PageCount

VARIABLES
    generation, \* The space's generation, advanced by each unmap.
    mapped,     \* Pages whose leaf is still present.
    freed,      \* Pages whose frame went back to its pool.
    inSet,      \* CPUs in the space's CPU set.
    current,    \* CPUs with the space loaded.
    hasSlot,    \* CPUs with a PCID slot for the space.
    seen,       \* The generation each CPU's slot has seen.
    cached,     \* The pages each CPU's TLB holds under the space's PCID.
    rpc,        \* Each requester's program counter.
    request,    \* The generation each requester's unmap created.
    pending     \* The CPUs each requester still waits for.

vars == << generation, mapped, freed, inSet, current, hasSlot, seen, cached, rpc, request,
           pending >>

Init ==
    /\ generation = 0
    /\ mapped = Pages
    /\ freed = {}
    /\ inSet = {}
    /\ current = {}
    /\ hasSlot = {}
    /\ seen = [c \in Cpus |-> 0]
    /\ cached = [c \in Cpus |-> {}]
    /\ rpc = [p \in Pages |-> "idle"]
    /\ request = [p \in Pages |-> 0]
    /\ pending = [p \in Pages |-> {}]

\* `tlb_generation::catch_up` with a range extent naming page `page`.
CatchUpSeen(s, rq, latest) ==
    LET newest == IF rq > latest THEN rq ELSE latest
    IN IF s >= newest \/ rq <= s THEN s
       ELSE IF rq = s + 1 /\ rq = newest THEN rq
       ELSE newest

CatchUpCached(s, rq, latest, page, pages) ==
    LET newest == IF rq > latest THEN rq ELSE latest
    IN IF s >= newest \/ rq <= s THEN pages
       ELSE IF rq = s + 1 /\ rq = newest THEN pages \ {page}
       ELSE {}

Access(c, p) ==
    /\ c \in current
    /\ c \notin {r \in Pages: rpc[r] = "waiting"}
    /\ p \in mapped
    /\ p \notin cached[c]
    /\ cached' = [cached EXCEPT ![c] = @ \cup {p}]
    /\ UNCHANGED << generation, mapped, freed, inSet, current, hasSlot, seen, rpc, request,
                    pending >>

Join(c) ==
    /\ c \notin inSet
    /\ inSet' = inSet \cup {c}
    /\ UNCHANGED << generation, mapped, freed, current, hasSlot, seen, cached, rpc, request,
                    pending >>

\* `Slots::load`: a fresh slot and a stale one load with a flush, a current one without.
Load(c) ==
    /\ c \in inSet
    /\ c \notin current
    /\ current' = current \cup {c}
    /\ hasSlot' = hasSlot \cup {c}
    /\ seen' = [seen EXCEPT ![c] = IF c \in hasSlot /\ @ >= generation THEN @ ELSE generation]
    /\ cached' = [cached EXCEPT ![c] =
                    IF c \notin hasSlot THEN {}
                    ELSE IF seen[c] < generation /\ StaleSlotFlushes THEN {}
                    ELSE @]
    /\ UNCHANGED << generation, mapped, freed, inSet, rpc, request, pending >>

SwitchOut(c) ==
    /\ c \in current
    /\ c \notin {r \in Pages: rpc[r] = "waiting"}
    /\ current' = current \ {c}
    /\ UNCHANGED << generation, mapped, freed, inSet, hasSlot, seen, cached, rpc, request,
                    pending >>

\* The slot is reused for another space with a flushing load, then the CPU leaves the set.
Evict(c) ==
    /\ c \in inSet
    /\ c \notin current
    /\ inSet' = inSet \ {c}
    /\ hasSlot' = hasSlot \ {c}
    /\ cached' = [cached EXCEPT ![c] = {}]
    /\ UNCHANGED << generation, mapped, freed, current, seen, rpc, request, pending >>

\* Under the space lock: clear the leaf, advance the generation, snapshot the set, flush locally.
Unmap(r) ==
    /\ rpc[r] = "idle"
    /\ generation' = generation + 1
    /\ mapped' = mapped \ {r}
    /\ request' = [request EXCEPT ![r] = generation + 1]
    /\ pending' = [pending EXCEPT ![r] = inSet \ {r}]
    /\ rpc' = [rpc EXCEPT ![r] = "waiting"]
    /\ IF r \in current
       THEN /\ seen' = [seen EXCEPT ![r] = CatchUpSeen(@, generation + 1, generation + 1)]
            /\ cached' = [cached EXCEPT ![r] =
                            CatchUpCached(seen[r], generation + 1, generation + 1, r, @)]
       ELSE UNCHANGED << seen, cached >>
    /\ UNCHANGED << freed, inSet, current, hasSlot >>

\* The IPI handler takes no lock: a loaded space catches up, any other acknowledges stale.
Handle(d, r) ==
    /\ d \in pending[r]
    /\ IF d \in current
       THEN /\ seen' = [seen EXCEPT ![d] = CatchUpSeen(@, request[r], generation)]
            /\ cached' = [cached EXCEPT ![d] = CatchUpCached(seen[d], request[r], generation, r, @)]
       ELSE UNCHANGED << seen, cached >>
    /\ pending' = [pending EXCEPT ![r] = @ \ {d}]
    /\ UNCHANGED << generation, mapped, freed, inSet, current, hasSlot, rpc, request >>

Complete(r) ==
    /\ rpc[r] = "waiting"
    /\ pending[r] = {}
    /\ freed' = freed \cup {r}
    /\ rpc' = [rpc EXCEPT ![r] = "done"]
    /\ UNCHANGED << generation, mapped, inSet, current, hasSlot, seen, cached, request, pending >>

Next ==
    \/ \E c \in Cpus, p \in Pages: Access(c, p)
    \/ \E c \in Cpus: Join(c) \/ Load(c) \/ SwitchOut(c) \/ Evict(c)
    \/ \E r \in Pages: Unmap(r) \/ Complete(r)
    \/ \E d \in Cpus, r \in Pages: Handle(d, r)

Spec ==
    /\ Init
    /\ [][Next]_vars
    /\ \A d \in Cpus, r \in Pages: WF_vars(Handle(d, r))
    /\ \A r \in Pages: WF_vars(Complete(r))

\* Shootdown: a frame returns to its pool only after every CPU that could hold its translation
\* acknowledged the flush or holds the space's slot marked stale below the flush's generation.
FreedOnlyWhenFlushed ==
    \A p \in freed, c \in Cpus:
        p \in cached[c] => c \notin current /\ seen[c] < request[p]

\* Shootdown: no CPU waits for an acknowledgement with interrupts off, so every wait ends.
FlushesComplete == \A r \in Pages: rpc[r] = "waiting" ~> rpc[r] = "done"

CachedOnlyUnderASlot == \A c \in Cpus: cached[c] # {} => c \in hasSlot /\ c \in inSet

TypeOK ==
    /\ generation \in 0..PageCount
    /\ mapped \subseteq Pages
    /\ freed \subseteq Pages \ mapped
    /\ inSet \subseteq Cpus
    /\ current \subseteq inSet
    /\ hasSlot \subseteq inSet
    /\ seen \in [Cpus -> 0..PageCount]
    /\ cached \in [Cpus -> SUBSET Pages]
    /\ rpc \in [Pages -> {"idle", "waiting", "done"}]
    /\ request \in [Pages -> 0..PageCount]
    /\ pending \in [Pages -> SUBSET Cpus]

=============================================================================
