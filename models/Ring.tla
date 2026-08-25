-------------------------------- MODULE Ring --------------------------------
(***************************************************************************)
(* The submission/completion protocol of D-79, checked before the kernel    *)
(* implements it.                                                          *)
(*                                                                         *)
(* The model is deliberately about *admission and accounting*, not about    *)
(* index arithmetic: `libs/ring-proto` holds the arithmetic and proptest    *)
(* covers it over sequences no hand-written test would reach. What cannot   *)
(* be reached that way is the claim that no reachable state of the whole    *)
(* protocol drops a completion — a statement about every interleaving of    *)
(* four actors, which is what a model checker is for (D-84).               *)
(*                                                                         *)
(* Two of the four actors are userspace and are therefore *hostile*. The    *)
(* consumer of the completion queue reaps at times of its choosing or never *)
(* at all, and the submitter offers work whenever it likes. The model gives *)
(* them no fairness, so every property below must hold against a userspace  *)
(* that stops cooperating at the worst moment.                             *)
(***************************************************************************)
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

(***************************************************************************)
(* Userspace offers work. Admission is the whole of the design: the         *)
(* reservation is taken *here*, before the operation is accepted, so an     *)
(* operation that is admitted always has somewhere to report. This is       *)
(* Windows RIO's model. io_uring's `CQ_NODROP` instead accepts the work and *)
(* takes an unbounded kernel-memory liability for a consumer that never     *)
(* reaps.                                                                   *)
(***************************************************************************)
Submit ==
    /\ admitted < MaxOperations
    /\ submitted < SubmissionSlots
    /\ reserved + posted < CompletionSlots
    /\ submitted' = submitted + 1
    /\ reserved'  = reserved + 1
    /\ admitted'  = admitted + 1
    /\ UNCHANGED << inflight, posted, reported >>

(***************************************************************************)
(* The kernel takes a submission. The reservation stays held: the operation *)
(* is in flight and the slot it will report into is still its own.          *)
(***************************************************************************)
Take ==
    /\ submitted > 0
    /\ submitted' = submitted - 1
    /\ inflight'  = inflight + 1
    /\ UNCHANGED << posted, reserved, admitted, reported >>

(***************************************************************************)
(* The kernel completes an operation. Exactly one terminal completion per   *)
(* operation, always — cancellation is advisory and still posts one, which  *)
(* is why there is no third outcome here to model.                          *)
(***************************************************************************)
Complete ==
    /\ inflight > 0
    /\ inflight' = inflight - 1
    /\ posted'   = posted + 1
    /\ reserved' = reserved - 1
    /\ reported' = reported + 1
    /\ UNCHANGED << submitted, admitted >>

(***************************************************************************)
(* Userspace reaps a completion, at a time of its choosing or never.        *)
(***************************************************************************)
Reap ==
    /\ posted > 0
    /\ posted' = posted - 1
    /\ UNCHANGED << submitted, inflight, reserved, admitted, reported >>

Next == Submit \/ Take \/ Complete \/ Reap

(* No fairness: userspace is not obliged to do anything, ever. *)
Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------

(* `inflight` is bounded by the *completion* queue, not the submission one:  *)
(* operations taken from the SQ stay in flight while more are submitted, so  *)
(* what limits them is the reservation each holds. The first version of this *)
(* invariant said `SubmissionSlots` and TLC found the eleven-step trace that *)
(* reaches five in flight with a queue of four (D-112).                      *)
TypeOK ==
    /\ submitted \in 0..SubmissionSlots
    /\ inflight  \in 0..CompletionSlots
    /\ posted    \in 0..CompletionSlots
    /\ reserved  \in 0..CompletionSlots
    /\ admitted  \in 0..MaxOperations
    /\ reported  \in 0..MaxOperations

(***************************************************************************)
(* The property the whole design exists for: a completion is never dropped. *)
(* Every operation that has not yet reported holds a reservation, and every *)
(* completion already posted holds a slot, so the two together can never    *)
(* exceed the completion queue.                                            *)
(***************************************************************************)
NoDroppedCompletion == reserved + posted <= CompletionSlots

(***************************************************************************)
(* The accounting the kernel's `outstanding()` reports: a reservation is    *)
(* held by exactly the operations that have not reported.                   *)
(***************************************************************************)
ReservationsAreOperations == reserved = submitted + inflight

(***************************************************************************)
(* Exactly one terminal completion per operation, in the only form a safety *)
(* property can take: never more completions than operations admitted.      *)
(***************************************************************************)
OneCompletionPerOperation == reported <= admitted

(***************************************************************************)
(* Nothing is lost in the middle: every admitted operation is either        *)
(* waiting, in flight, or reported.                                        *)
(***************************************************************************)
NothingVanishes == admitted = submitted + inflight + reported

=============================================================================
