-------------------------------- MODULE Call --------------------------------
\* The call cell of `libs/channel-state` over successive calls of one caller to one server.
\* Each call moves one handle to the server and each reply moves one back. The inbox entry
\* names the call's generation, the server's reply right names the generation it received, and
\* every event on the cell names a generation; `Cell::next` refuses a mismatch as stale.
\* Racing: reply, close of the reply right, cancel, timeout, server death, caller death.
EXTENDS Naturals, Sequences, FiniteSets

CONSTANTS
    Calls,              \* Successive calls on one cell, so a stale right can meet a later call.
    GenerationChecked   \* A reply's claim names its right's generation; FALSE is the mutation.

ASSUME Calls \in Nat \ {0}
ASSUME GenerationChecked \in BOOLEAN

Generations == 0..(Calls - 1)
Outcomes == {"Replied", "Abandoned", "Cancelled", "TimedOut", "PeerClosed"}
States == {"Idle", "Queued", "Delivered", "Replying"} \cup Outcomes
CallHandle(g) == <<"call", g>>
ReplyHandle(g) == <<"reply", g>>
Handles == {CallHandle(g): g \in Generations} \cup {ReplyHandle(g): g \in Generations}
NoReply == Calls

\* `CallState::next`: the successor, or "none" where the crate answers `Unexpected`.
Step(s, e) ==
    CASE s = "Idle" /\ e = "Commit" -> "Queued"
      [] s = "Queued" /\ e = "Deliver" -> "Delivered"
      [] s = "Queued" /\ e = "Close" -> "PeerClosed"
      [] s \in {"Queued", "Delivered"} /\ e = "Cancel" -> "Cancelled"
      [] s \in {"Queued", "Delivered"} /\ e = "Timeout" -> "TimedOut"
      [] s = "Delivered" /\ e = "Claim" -> "Replying"
      [] s = "Delivered" /\ e = "Abandon" -> "Abandoned"
      [] s = "Replying" /\ e = "Complete" -> "Replied"
      [] s \in Outcomes /\ e = "Consume" -> "Idle"
      [] OTHER -> "none"

VARIABLES
    state,          \* The cell's state.
    generation,     \* The cell's generation; the call it holds or will hold.
    inbox,          \* The endpoint's queued call entries, each the generation it names.
    callerTable,    \* Handles in the caller's table.
    serverTable,    \* Handles in the server's table.
    staging,        \* Reply handles staged in the caller's cell.
    closed,         \* Handles destroyed with their entry, table or cell.
    rights,         \* Generations named by reply rights in the server's table.
    replying,       \* The right whose claim succeeded and whose reply is being copied.
    callerAlive,
    serverAlive,
    received,       \* Calls the server received.
    outcomes,       \* Outcomes reached per call.
    outcome,        \* The outcome each call reached.
    replies,        \* Successful replies per reply right.
    stray           \* A reply took effect on a call other than the one its right names.

vars == << state, generation, inbox, callerTable, serverTable, staging, closed, rights, replying,
           callerAlive, serverAlive, received, outcomes, outcome, replies, stray >>

Init ==
    /\ state = "Idle"
    /\ generation = 0
    /\ inbox = << >>
    /\ callerTable = {CallHandle(g): g \in Generations}
    /\ serverTable = {ReplyHandle(g): g \in Generations}
    /\ staging = {}
    /\ closed = {}
    /\ rights = {}
    /\ replying = NoReply
    /\ callerAlive = TRUE
    /\ serverAlive = TRUE
    /\ received = {}
    /\ outcomes = [g \in Generations |-> 0]
    /\ outcome = [g \in Generations |-> "none"]
    /\ replies = [g \in Generations |-> 0]
    /\ stray = FALSE

\* `Cell::next`: `event` names call `g`; on success the cell moves and an outcome is counted.
Admits(e, g) == g = generation /\ Step(state, e) # "none"

Apply(e) ==
    /\ state' = Step(state, e)
    /\ IF Step(state, e) \in Outcomes
       THEN /\ outcomes' = [outcomes EXCEPT ![generation] = @ + 1]
            /\ outcome' = [outcome EXCEPT ![generation] = Step(state, e)]
       ELSE UNCHANGED << outcomes, outcome >>

\* An event that `Cell::next` refuses leaves the cell alone.
Try(e, g) ==
    IF Admits(e, g) THEN Apply(e) ELSE UNCHANGED << state, outcomes, outcome >>

Commit ==
    /\ callerAlive /\ serverAlive
    /\ state = "Idle"
    /\ generation < Calls
    /\ Apply("Commit")
    /\ inbox' = Append(inbox, generation)
    /\ callerTable' = callerTable \ {CallHandle(generation)}
    /\ UNCHANGED << generation, serverTable, staging, closed, rights, replying, callerAlive,
                    serverAlive, received, replies, stray >>

Cancel ==
    /\ Admits("Cancel", generation)
    /\ Apply("Cancel")
    /\ UNCHANGED << generation, inbox, callerTable, serverTable, staging, closed, rights,
                    replying, callerAlive, serverAlive, received, replies, stray >>

Timeout ==
    /\ callerAlive
    /\ Admits("Timeout", generation)
    /\ Apply("Timeout")
    /\ UNCHANGED << generation, inbox, callerTable, serverTable, staging, closed, rights,
                    replying, callerAlive, serverAlive, received, replies, stray >>

\* The caller installs the staged reply handles, then the cell is Idle at the next generation.
Consume ==
    /\ callerAlive
    /\ Admits("Consume", generation)
    /\ state' = "Idle"
    /\ generation' = generation + 1
    /\ callerTable' = callerTable \cup staging
    /\ staging' = {}
    /\ UNCHANGED << inbox, serverTable, closed, rights, replying, callerAlive, serverAlive,
                    received, outcomes, outcome, replies, stray >>

\* A killed caller's wait ends cancelled (its own `Cancel`); teardown then destroys its table and
\* any staged reply once the cell holds no reply in progress.
CallerDies ==
    /\ callerAlive
    /\ callerAlive' = FALSE
    /\ UNCHANGED << state, generation, inbox, callerTable, serverTable, staging, closed, rights,
                    replying, serverAlive, received, outcomes, outcome, replies, stray >>

CallerReclaim ==
    /\ ~callerAlive
    /\ state # "Replying"
    /\ callerTable \cup staging # {}
    /\ closed' = closed \cup callerTable \cup staging
    /\ callerTable' = {}
    /\ staging' = {}
    /\ UNCHANGED << state, generation, inbox, serverTable, rights, replying, callerAlive,
                    serverAlive, received, outcomes, outcome, replies, stray >>

\* The server reads the head entry; a call no longer Queued is discarded with its handle.
Receive ==
    /\ serverAlive
    /\ inbox # << >>
    /\ LET g == Head(inbox)
       IN /\ Try("Deliver", g)
          /\ IF Admits("Deliver", g)
             THEN /\ serverTable' = serverTable \cup {CallHandle(g)}
                  /\ rights' = rights \cup {g}
                  /\ received' = received \cup {g}
                  /\ UNCHANGED closed
             ELSE /\ closed' = closed \cup {CallHandle(g)}
                  /\ UNCHANGED << serverTable, rights, received >>
    /\ inbox' = Tail(inbox)
    /\ UNCHANGED << generation, callerTable, staging, replying, callerAlive, serverAlive,
                    outcomes, outcome, replies, stray >>

\* `Reply` spends the right either way; a claim refused by the cell moves nothing.
Claim(g) ==
    /\ serverAlive
    /\ replying = NoReply
    /\ g \in rights
    /\ rights' = rights \ {g}
    /\ LET named == IF GenerationChecked THEN g ELSE generation
       IN IF Admits("Claim", named)
          THEN /\ Apply("Claim")
               /\ replying' = g
               /\ stray' = (stray \/ g # generation)
          ELSE UNCHANGED << state, outcomes, outcome, replying, stray >>
    /\ UNCHANGED << generation, inbox, callerTable, serverTable, staging, closed, callerAlive,
                    serverAlive, received, replies >>

\* Copy and stage the reply, then Replying to Replied; nothing else admits Replying.
Complete ==
    /\ replying # NoReply
    /\ state = "Replying"
    /\ Apply("Complete")
    /\ serverTable' = serverTable \ {ReplyHandle(replying)}
    /\ staging' = staging \cup {ReplyHandle(replying)}
    /\ replies' = [replies EXCEPT ![replying] = @ + 1]
    /\ replying' = NoReply
    /\ UNCHANGED << generation, inbox, callerTable, closed, rights, callerAlive, serverAlive,
                    received, stray >>

\* Closing a reply right, by the server or by its teardown, abandons the call it names.
CloseRight(g) ==
    /\ replying = NoReply
    /\ g \in rights
    /\ rights' = rights \ {g}
    /\ Try("Abandon", g)
    /\ UNCHANGED << generation, inbox, callerTable, serverTable, staging, closed, replying,
                    callerAlive, serverAlive, received, replies, stray >>

\* A kill takes effect on return to ring 3, so a claimed reply finishes first.
ServerDies ==
    /\ serverAlive
    /\ replying = NoReply
    /\ serverAlive' = FALSE
    /\ UNCHANGED << state, generation, inbox, callerTable, serverTable, staging, closed, rights,
                    replying, callerAlive, received, outcomes, outcome, replies, stray >>

\* The destroyed endpoint closes each queued call and destroys its entry.
DestroyEntry ==
    /\ ~serverAlive
    /\ inbox # << >>
    /\ Try("Close", Head(inbox))
    /\ closed' = closed \cup {CallHandle(Head(inbox))}
    /\ inbox' = Tail(inbox)
    /\ UNCHANGED << generation, callerTable, serverTable, staging, rights, replying, callerAlive,
                    serverAlive, received, replies, stray >>

ServerReclaim ==
    /\ ~serverAlive
    /\ rights = {}
    /\ serverTable # {}
    /\ closed' = closed \cup serverTable
    /\ serverTable' = {}
    /\ UNCHANGED << state, generation, inbox, callerTable, staging, rights, replying, callerAlive,
                    serverAlive, received, outcomes, outcome, replies, stray >>

Next ==
    \/ Commit \/ Cancel \/ Timeout \/ Consume \/ CallerDies \/ CallerReclaim
    \/ Receive \/ Complete \/ ServerDies \/ DestroyEntry \/ ServerReclaim
    \/ \E g \in Generations: Claim(g) \/ CloseRight(g)

\* Deadlines always fire, and a claimed reply always finishes.
Spec == Init /\ [][Next]_vars /\ WF_vars(Timeout) /\ WF_vars(Cancel) /\ WF_vars(Complete)

\* Call: each call has at most one outcome, and every committed call reaches one.
OneOutcome == \A g \in Generations: outcomes[g] <= 1

Committed(g) == g < generation \/ (g = generation /\ state # "Idle")

EveryCallEnds == \A g \in Generations: Committed(g) ~> outcomes[g] = 1

\* Call: a reply right names one call and succeeds at most once.
ReplyOnce == ~stray /\ \A g \in Generations: replies[g] <= 1

\* Call: a call the server received is never reported as peer closed.
ReceivedNeverPeerClosed == \A g \in received: outcome[g] # "PeerClosed"

\* Conservation: a moved handle is in exactly one place at every step.
InboxHandles == {CallHandle(inbox[i]): i \in 1..Len(inbox)}
Places == << callerTable, InboxHandles, serverTable, staging, closed >>
Conservation ==
    \A h \in Handles: Cardinality({i \in 1..5: h \in Places[i]}) = 1

TypeOK ==
    /\ state \in States
    /\ generation \in 0..Calls
    /\ \A i \in 1..Len(inbox): inbox[i] \in Generations
    /\ rights \subseteq Generations
    /\ replying \in Generations \cup {NoReply}
    /\ outcomes \in [Generations -> 0..2]
    /\ replies \in [Generations -> 0..2]

=============================================================================
