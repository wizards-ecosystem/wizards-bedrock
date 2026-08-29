---------------------------- MODULE RingMemory ----------------------------
(***************************************************************************)
(* The shared-memory and terminal-ownership composition of D-229.          *)
(*                                                                         *)
(* `Ring.tla` keeps admission capacity deliberately anonymous. This model  *)
(* gives two operations identities and carries the D-221 release/acquire   *)
(* publication edge through admission, normal completion, cancellation,   *)
(* retirement and userspace reaping. The same message state is reset from  *)
(* submission to completion, so both queue directions take the same weak-  *)
(* memory path.                                                            *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
    OperationCount,
    CompletionSlots,
    ReleaseOrders,
    AcquireOrders,
    ReserveOnAdmit,
    SingleTerminal

ASSUME OperationCount \in Nat \ {0}
ASSUME CompletionSlots \in Nat \ {0}
ASSUME CompletionSlots >= OperationCount
ASSUME ReleaseOrders \in BOOLEAN
ASSUME AcquireOrders \in BOOLEAN
ASSUME ReserveOnAdmit \in BOOLEAN
ASSUME SingleTerminal \in BOOLEAN

Operations == 1..OperationCount
Kinds == {"submission", "completion", "done"}
Causes == {"none", "normal", "cancel", "retire"}

VARIABLES
    kind,
    payloadIssued,
    payloadVisible,
    tailIssued,
    tailVisible,
    tailRead,
    payloadRead,
    staleReads,
    admitted,
    live,
    settling,
    cause,
    reservations,
    posted,
    reportCount,
    reported,
    reaped

vars == <<
    kind,
    payloadIssued,
    payloadVisible,
    tailIssued,
    tailVisible,
    tailRead,
    payloadRead,
    staleReads,
    admitted,
    live,
    settling,
    cause,
    reservations,
    posted,
    reportCount,
    reported,
    reaped
>>

Init ==
    /\ kind           = [op \in Operations |-> "submission"]
    /\ payloadIssued  = [op \in Operations |-> FALSE]
    /\ payloadVisible = [op \in Operations |-> FALSE]
    /\ tailIssued     = [op \in Operations |-> FALSE]
    /\ tailVisible    = [op \in Operations |-> FALSE]
    /\ tailRead       = [op \in Operations |-> FALSE]
    /\ payloadRead    = [op \in Operations |-> FALSE]
    /\ staleReads     = {}
    /\ admitted       = {}
    /\ live           = {}
    /\ settling       = {}
    /\ cause          = [op \in Operations |-> "none"]
    /\ reservations   = 0
    /\ posted         = 0
    /\ reportCount    = [op \in Operations |-> 0]
    /\ reported       = 0
    /\ reaped         = 0

(***************************************************************************)
(* Producer program order issues the payload before its tail. Visibility   *)
(* remains independent until the release edge below constrains it.         *)
(***************************************************************************)
IssuePayload(op) ==
    /\ kind[op] \in {"submission", "completion"}
    /\ ~payloadIssued[op]
    /\ (kind[op] = "submission" \/ op \in settling)
    /\ payloadIssued' = [payloadIssued EXCEPT ![op] = TRUE]
    /\ UNCHANGED <<
        kind, payloadVisible, tailIssued, tailVisible, tailRead,
        payloadRead, staleReads, admitted, live, settling, cause,
        reservations, posted, reportCount, reported, reaped
        >>

MakePayloadVisible(op) ==
    /\ payloadIssued[op]
    /\ ~payloadVisible[op]
    /\ payloadVisible' = [payloadVisible EXCEPT ![op] = TRUE]
    /\ UNCHANGED <<
        kind, payloadIssued, tailIssued, tailVisible, tailRead,
        payloadRead, staleReads, admitted, live, settling, cause,
        reservations, posted, reportCount, reported, reaped
        >>

IssueSubmissionTail(op) ==
    /\ kind[op] = "submission"
    /\ payloadIssued[op]
    /\ ~tailIssued[op]
    /\ tailIssued' = [tailIssued EXCEPT ![op] = TRUE]
    /\ UNCHANGED <<
        kind, payloadIssued, payloadVisible, tailVisible, tailRead,
        payloadRead, staleReads, admitted, live, settling, cause,
        reservations, posted, reportCount, reported, reaped
        >>

(***************************************************************************)
(* A terminal tail is the commit point: until it is issued the operation   *)
(* and its reservation remain owned. `SingleTerminal = FALSE` is the       *)
(* deliberate duplicate-publication mutation.                              *)
(***************************************************************************)
IssueCompletionTail(op) ==
    /\ kind[op] = "completion"
    /\ payloadIssued[op]
    /\ cause[op] # "none"
    /\ (~tailIssued[op] \/ ~SingleTerminal)
    /\ posted < CompletionSlots
    /\ tailIssued' = [tailIssued EXCEPT ![op] = TRUE]
    /\ live' = live \ {op}
    /\ settling' = settling \ {op}
    /\ reservations' = IF op \in live /\ reservations > 0
                        THEN reservations - 1
                        ELSE reservations
    /\ posted' = posted + 1
    /\ reportCount' = [reportCount EXCEPT ![op] = @ + 1]
    /\ reported' = reported + 1
    /\ UNCHANGED <<
        kind, payloadIssued, payloadVisible, tailVisible, tailRead,
        payloadRead, staleReads, admitted, cause, reaped
        >>

(***************************************************************************)
(* Release orders visibility, not issue. Removing this dependency permits  *)
(* the peer to observe a new tail while the payload is still old.          *)
(***************************************************************************)
MakeTailVisible(op) ==
    /\ tailIssued[op]
    /\ ~tailVisible[op]
    /\ (~ReleaseOrders \/ payloadVisible[op])
    /\ tailVisible' = [tailVisible EXCEPT ![op] = TRUE]
    /\ UNCHANGED <<
        kind, payloadIssued, payloadVisible, tailIssued, tailRead,
        payloadRead, staleReads, admitted, live, settling, cause,
        reservations, posted, reportCount, reported, reaped
        >>

AcquireTail(op) ==
    /\ tailVisible[op]
    /\ ~tailRead[op]
    /\ tailRead' = [tailRead EXCEPT ![op] = TRUE]
    /\ UNCHANGED <<
        kind, payloadIssued, payloadVisible, tailIssued, tailVisible,
        payloadRead, staleReads, admitted, live, settling, cause,
        reservations, posted, reportCount, reported, reaped
        >>

(***************************************************************************)
(* Without acquire the compiler/CPU may perform the payload read early.     *)
(* It becomes an actual stale observation only if the tail is later read.  *)
(***************************************************************************)
ReadPayload(op) ==
    /\ kind[op] \in {"submission", "completion"}
    /\ ~payloadRead[op]
    /\ (~AcquireOrders \/ tailRead[op])
    /\ payloadRead' = [payloadRead EXCEPT ![op] = TRUE]
    /\ staleReads' = IF payloadVisible[op]
                     THEN staleReads
                     ELSE staleReads \cup {op}
    /\ UNCHANGED <<
        kind, payloadIssued, payloadVisible, tailIssued, tailVisible,
        tailRead, admitted, live, settling, cause, reservations, posted,
        reportCount, reported, reaped
        >>

(***************************************************************************)
(* Admission and reservation are one transition. The message state is      *)
(* reset for the completion which this operation is now owed.              *)
(***************************************************************************)
Admit(op) ==
    /\ kind[op] = "submission"
    /\ tailRead[op]
    /\ payloadRead[op]
    /\ op \notin admitted
    /\ reservations + posted < CompletionSlots
    /\ kind' = [kind EXCEPT ![op] = "completion"]
    /\ payloadIssued' = [payloadIssued EXCEPT ![op] = FALSE]
    /\ payloadVisible' = [payloadVisible EXCEPT ![op] = FALSE]
    /\ tailIssued' = [tailIssued EXCEPT ![op] = FALSE]
    /\ tailVisible' = [tailVisible EXCEPT ![op] = FALSE]
    /\ tailRead' = [tailRead EXCEPT ![op] = FALSE]
    /\ payloadRead' = [payloadRead EXCEPT ![op] = FALSE]
    /\ staleReads' = staleReads \ {op}
    /\ admitted' = admitted \cup {op}
    /\ live' = live \cup {op}
    /\ reservations' = reservations + IF ReserveOnAdmit THEN 1 ELSE 0
    /\ UNCHANGED <<
        settling, cause, posted, reportCount, reported, reaped
        >>

BeginSettlement(op, why) ==
    /\ why \in Causes \ {"none"}
    /\ kind[op] = "completion"
    /\ op \in live
    /\ op \notin settling
    /\ cause[op] = "none"
    /\ settling' = settling \cup {op}
    /\ cause' = [cause EXCEPT ![op] = why]
    /\ UNCHANGED <<
        kind, payloadIssued, payloadVisible, tailIssued, tailVisible,
        tailRead, payloadRead, staleReads, admitted, live, reservations,
        posted, reportCount, reported, reaped
        >>

BeginNormal(op) == BeginSettlement(op, "normal")
BeginCancel(op) == BeginSettlement(op, "cancel")
BeginRetire(op) == BeginSettlement(op, "retire")

Reap(op) ==
    /\ kind[op] = "completion"
    /\ reportCount[op] > 0
    /\ tailRead[op]
    /\ payloadRead[op]
    /\ posted > 0
    /\ kind' = [kind EXCEPT ![op] = "done"]
    /\ posted' = posted - 1
    /\ reaped' = reaped + 1
    /\ UNCHANGED <<
        payloadIssued, payloadVisible, tailIssued, tailVisible, tailRead,
        payloadRead, staleReads, admitted, live, settling, cause,
        reservations, reportCount, reported
        >>

Next ==
    \E op \in Operations:
        IssuePayload(op)
        \/ MakePayloadVisible(op)
        \/ IssueSubmissionTail(op)
        \/ IssueCompletionTail(op)
        \/ MakeTailVisible(op)
        \/ AcquireTail(op)
        \/ ReadPayload(op)
        \/ Admit(op)
        \/ BeginNormal(op)
        \/ BeginCancel(op)
        \/ BeginRetire(op)
        \/ Reap(op)

Spec == Init /\ [][Next]_vars

(***************************************************************************)
(* A speculative/early payload read is harmless only until a new tail is   *)
(* observed. Release plus acquire makes their conjunction unreachable.     *)
(***************************************************************************)
NoStalePayload ==
    \A op \in Operations: ~(tailRead[op] /\ op \in staleReads)

ReservationsAreLive == reservations = Cardinality(live)

NoDroppedCompletion == reservations + posted <= CompletionSlots

OneCompletionPerOperation ==
    \A op \in Operations: reportCount[op] <= 1

NothingVanishes == Cardinality(admitted) = Cardinality(live) + reported

EveryReportWasAdmitted ==
    \A op \in Operations: reportCount[op] > 0 => op \in admitted

SettlementHasOneCause ==
    /\ \A op \in settling: cause[op] \in Causes \ {"none"}
    /\ \A op \in Operations:
        cause[op] # "none" => op \in settling \/ reportCount[op] > 0

TypeOK ==
    /\ kind \in [Operations -> Kinds]
    /\ payloadIssued \in [Operations -> BOOLEAN]
    /\ payloadVisible \in [Operations -> BOOLEAN]
    /\ tailIssued \in [Operations -> BOOLEAN]
    /\ tailVisible \in [Operations -> BOOLEAN]
    /\ tailRead \in [Operations -> BOOLEAN]
    /\ payloadRead \in [Operations -> BOOLEAN]
    /\ staleReads \subseteq Operations
    /\ admitted \subseteq Operations
    /\ live \subseteq admitted
    /\ settling \subseteq live
    /\ cause \in [Operations -> Causes]
    /\ reservations \in 0..CompletionSlots
    /\ posted \in 0..CompletionSlots
    /\ reportCount \in [Operations -> 0..CompletionSlots]
    /\ reported \in 0..CompletionSlots
    /\ reaped \in 0..CompletionSlots

=============================================================================
