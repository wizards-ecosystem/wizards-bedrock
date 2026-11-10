-------------------------------- MODULE Ring --------------------------------
\* No reachable state of ring admission and accounting drops a completion, over every
\* interleaving of four actors; `libs/ring-proto` holds the index arithmetic.
\* The two userspace actors are hostile and have no fairness, so every property must hold
\* against a userspace that stops cooperating at the worst moment.
EXTENDS Naturals

CONSTANTS
    SubmissionSlots,    \* Entries in the submission queue.
    CompletionSlots,    \* Entries in the completion queue: >= 2 * SubmissionSlots.
    MaxOperations       \* Bound on the run, so the state space is finite.

ASSUME SubmissionSlots \in Nat \ {0}
ASSUME CompletionSlots >= 2 * SubmissionSlots
ASSUME MaxOperations \in Nat

VARIABLES
    submitted,   \* Submissions posted and not yet taken by the kernel.
    inflight,    \* Operations the kernel has taken and not yet completed.
    posted,      \* Completions posted and not yet reaped by userspace.
    reserved,    \* Completion slots spoken for: inflight + submitted + posted.
    admitted,    \* Total operations ever admitted.
    reported     \* Total completions ever posted.

vars == << submitted, inflight, posted, reserved, admitted, reported >>

Init ==
    /\ submitted = 0
    /\ inflight  = 0
    /\ posted    = 0
    /\ reserved  = 0
    /\ admitted  = 0
    /\ reported  = 0

\* The reservation is taken here, before the operation is accepted, so an admitted operation
\* always has somewhere to report: Windows RIO's model.
Submit ==
    /\ admitted < MaxOperations
    /\ submitted < SubmissionSlots
    /\ reserved + posted < CompletionSlots
    /\ submitted' = submitted + 1
    /\ reserved'  = reserved + 1
    /\ admitted'  = admitted + 1
    /\ UNCHANGED << inflight, posted, reported >>

\* The kernel takes a submission; the reservation stays held while it is in flight.
Take ==
    /\ submitted > 0
    /\ submitted' = submitted - 1
    /\ inflight'  = inflight + 1
    /\ UNCHANGED << posted, reserved, admitted, reported >>

\* Exactly one terminal completion per operation; cancellation still posts one, so there is no
\* third outcome to model.
Complete ==
    /\ inflight > 0
    /\ inflight' = inflight - 1
    /\ posted'   = posted + 1
    /\ reserved' = reserved - 1
    /\ reported' = reported + 1
    /\ UNCHANGED << submitted, admitted >>

\* Userspace reaps a completion, at a time of its choosing or never.
Reap ==
    /\ posted > 0
    /\ posted' = posted - 1
    /\ UNCHANGED << submitted, inflight, reserved, admitted, reported >>

Next == Submit \/ Take \/ Complete \/ Reap

(* No fairness: userspace is not obliged to do anything, ever. *)
Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------

\* `inflight` is bounded by the completion queue: operations taken from the submission queue stay
\* in flight while more are submitted, each holding its reservation.
TypeOK ==
    /\ submitted \in 0..SubmissionSlots
    /\ inflight  \in 0..CompletionSlots
    /\ posted    \in 0..CompletionSlots
    /\ reserved  \in 0..CompletionSlots
    /\ admitted  \in 0..MaxOperations
    /\ reported  \in 0..MaxOperations

\* A completion is never dropped: unreported operations hold reservations and posted completions
\* hold slots, so together they never exceed the completion queue.
NoDroppedCompletion == reserved + posted <= CompletionSlots

\* The kernel's `outstanding()`: reservations are held by exactly the unreported operations.
ReservationsAreOperations == reserved = submitted + inflight

\* Exactly one terminal completion per operation, as a safety property: never more completions
\* than operations admitted.
OneCompletionPerOperation == reported <= admitted

\* Every admitted operation is waiting, in flight, or reported.
NothingVanishes == admitted = submitted + inflight + reported

=============================================================================
