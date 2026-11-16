------------------------------- MODULE Wakeup -------------------------------
\* The wake word of `libs/wake-state` against two wakers (a wake and a timeout) over successive
\* waits of one thread. Each waker reaches the thread through its own list, unlinks it under the
\* list's lock and claims later by compare-exchange, so a claim can arrive after its wait ended.
\* `on_cpu` stands for the switch record the waker must see released before it places the thread.
EXTENDS Naturals

CONSTANTS
    Waits,          \* Successive waits of the thread, so a late claim can meet a later wait.
    TaggedClaims    \* A claim names the wait sequence it unlinked; FALSE is the mutation.

ASSUME Waits \in Nat \ {0}
ASSUME TaggedClaims \in BOOLEAN

Wakers == {"wake", "timeout"}
States == {"Running", "Parking", "Woken", "Blocked", "Waking"}

VARIABLES
    state,      \* The wake word's state.
    sequence,   \* The wake word's wait sequence; Park advances it.
    pc,         \* The thread's program counter.
    onCpu,      \* The thread's switch record is still in use on a CPU.
    queued,     \* The thread sits on a run queue.
    linked,     \* The sequence each list holds the thread under, 0 when unlinked.
    wpc,        \* Each waker's program counter.
    held,       \* The sequence each waker unlinked.
    claims,     \* Successful claims per wait sequence.
    stray       \* A claim took effect on a wait other than the one it unlinked.

vars == << state, sequence, pc, onCpu, queued, linked, wpc, held, claims, stray >>

Init ==
    /\ state = "Running"
    /\ sequence = 0
    /\ pc = "running"
    /\ onCpu = TRUE
    /\ queued = FALSE
    /\ linked = [k \in Wakers |-> 0]
    /\ wpc = [k \in Wakers |-> "idle"]
    /\ held = [k \in Wakers |-> 0]
    /\ claims = [w \in 1..Waits |-> 0]
    /\ stray = FALSE

\* Under the object lock: link into both lists, then Park, which starts a new sequence.
Register ==
    /\ pc = "running"
    /\ sequence < Waits
    /\ state' = "Parking"
    /\ sequence' = sequence + 1
    /\ linked' = [k \in Wakers |-> sequence + 1]
    /\ pc' = "parking"
    /\ UNCHANGED << onCpu, queued, wpc, held, claims, stray >>

\* Under its own run-queue lock: Parking to Blocked, or Resume from Woken without switching.
Block ==
    /\ pc = "parking"
    /\ IF state = "Parking"
       THEN /\ state' = "Blocked"
            /\ pc' = "switching"
       ELSE /\ state = "Woken"
            /\ state' = "Running"
            /\ pc' = "finishing"
    /\ UNCHANGED << sequence, onCpu, queued, linked, wpc, held, claims, stray >>

\* The next thread on that CPU clears `on_cpu` with release.
SwitchOut ==
    /\ pc = "switching"
    /\ onCpu' = FALSE
    /\ pc' = "blocked"
    /\ UNCHANGED << state, sequence, queued, linked, wpc, held, claims, stray >>

Run ==
    /\ pc = "blocked"
    /\ queued
    /\ queued' = FALSE
    /\ onCpu' = TRUE
    /\ pc' = "finishing"
    /\ UNCHANGED << state, sequence, linked, wpc, held, claims, stray >>

\* Under each list's lock: unlink from whatever list still holds the thread.
Finish ==
    /\ pc = "finishing"
    /\ linked' = [k \in Wakers |-> 0]
    /\ pc' = "running"
    /\ UNCHANGED << state, sequence, onCpu, queued, wpc, held, claims, stray >>

Unlink(k) ==
    /\ wpc[k] = "idle"
    /\ linked[k] # 0
    /\ held' = [held EXCEPT ![k] = linked[k]]
    /\ linked' = [linked EXCEPT ![k] = 0]
    /\ wpc' = [wpc EXCEPT ![k] = "claiming"]
    /\ UNCHANGED << state, sequence, pc, onCpu, queued, claims, stray >>

Claimed(k) ==
    /\ claims' = [claims EXCEPT ![held[k]] = @ + 1]
    /\ stray' = (stray \/ held[k] # sequence)

Claim(k) ==
    /\ wpc[k] = "claiming"
    /\ IF state \in {"Parking", "Blocked"} /\ (~TaggedClaims \/ held[k] = sequence)
       THEN /\ state' = IF state = "Parking" THEN "Woken" ELSE "Waking"
            /\ wpc' = [wpc EXCEPT ![k] = IF state = "Parking" THEN "idle" ELSE "placing"]
            /\ Claimed(k)
       ELSE /\ wpc' = [wpc EXCEPT ![k] = "idle"]
            /\ UNCHANGED << state, claims, stray >>
    /\ UNCHANGED << sequence, pc, onCpu, queued, linked, held >>

\* Spin until `on_cpu` reads clear with acquire, then enqueue and store Running.
Place(k) ==
    /\ wpc[k] = "placing"
    /\ ~onCpu
    /\ state' = "Running"
    /\ queued' = TRUE
    /\ wpc' = [wpc EXCEPT ![k] = "idle"]
    /\ UNCHANGED << sequence, pc, onCpu, linked, held, claims, stray >>

Next ==
    \/ Register
    \/ Block
    \/ SwitchOut
    \/ Run
    \/ Finish
    \/ \E k \in Wakers: Unlink(k) \/ Claim(k) \/ Place(k)

Spec == Init /\ [][Next]_vars

Reachable == \E k \in Wakers:
    linked[k] = sequence \/ (wpc[k] = "claiming" /\ held[k] = sequence)

\* Wakeup: if a waker claims after registration the thread is not left blocked, and a blocked
\* thread is always linked where a waker can reach it.
NoLostWakeup ==
    pc = "blocked" =>
        \/ state = "Blocked" /\ Reachable
        \/ state = "Waking" /\ \E k \in Wakers: wpc[k] = "placing"
        \/ state = "Running" /\ queued

\* Wakeup: a wait completes once, and only by a claim on that wait.
OneCompletion == ~stray /\ \A w \in 1..Waits: claims[w] <= 1

PlacedOffCpu == queued => ~onCpu

TypeOK ==
    /\ state \in States
    /\ sequence \in 0..Waits
    /\ pc \in {"running", "parking", "switching", "blocked", "finishing"}
    /\ onCpu \in BOOLEAN
    /\ queued \in BOOLEAN
    /\ linked \in [Wakers -> 0..Waits]
    /\ wpc \in [Wakers -> {"idle", "claiming", "placing"}]
    /\ held \in [Wakers -> 0..Waits]
    /\ claims \in [1..Waits -> 0..2]
    /\ stray \in BOOLEAN

=============================================================================
