------------------------------- MODULE Rehome -------------------------------
\* The re-home of a channel endpoint into the world that reads it, over one channel with ends A
\* and B made in world 1. A re-home freezes the end's record, copies its inbox and link, places the
\* new home with the copy, unfreezes and forwards the old record, then walks to the peer: it waits
\* out a frozen record, follows a forward while its home still holds the copied link, relinks the
\* first open record it meets, and settles its home only from the copied link. Writes, closes,
\* reads and world deaths race it, and both ends may move at once and again once installed.
\* A forwarded record outlives every link that names it, a dead world's teardown relinks every name
\* of its forwarded records before destroying them, and a write waits out a freeze.
EXTENDS Naturals, FiniteSets

CONSTANTS
    WorldCount,         \* Worlds; the channel is made in world 1.
    MovesA,             \* Re-homes end A may make.
    MovesB,             \* Re-homes end B may make.
    Writes,             \* Entries each end may write.
    WaitBound,          \* Freezes one walk may wait out.
    FollowBound,        \* Forwards one walk may follow.
    ForwardedOutlives,  \* FALSE: a forwarded record goes with its last count, unrepointed.
    WriteWaitsFreeze,   \* FALSE is the mutation: a write commits into a frozen record.
    Recheck             \* FALSE is the mutation: a walk follows without rereading its home.

ASSUME WorldCount \in Nat \ {0}
ASSUME MovesA \in Nat /\ MovesB \in Nat /\ Writes \in Nat
ASSUME WaitBound \in Nat /\ FollowBound \in Nat
ASSUME ForwardedOutlives \in BOOLEAN /\ WriteWaitsFreeze \in BOOLEAN /\ Recheck \in BOOLEAN

Worlds == 1..WorldCount
Ends == {"A", "B"}
Other(e) == IF e = "A" THEN "B" ELSE "A"
Moves(e) == IF e = "A" THEN MovesA ELSE MovesB

\* Record <<e, i>> is end e's home after its i-th re-home.
Rec == UNION {{<<e, i>>: i \in 0..Moves(e)}: e \in Ends}
NoRec == <<"none", 0>>
Entries == {<<e, i>>: e \in Ends, i \in 1..Writes}

Open(r) == <<"open", r>>
Fwd(r) == <<"fwd", r>>
Closed == <<"closed", NoRec>>
NoLink == <<"none", NoRec>>
Links == {Open(r): r \in Rec} \cup {Fwd(r): r \in Rec} \cup {Closed, NoLink}
IsOpen(l) == l[1] = "open"
IsFwd(l) == l[1] = "fwd"
Target(l) == l[2]

VARIABLES
    worldState, \* Each world: Live, Dying (killed, pins refused), Dead (dissolved).
    home,       \* The world each record was placed in; 0 before it exists.
    link,       \* Each record's `State.link`.
    frozen,     \* Each record's `State.frozen`.
    inbox,      \* Each record's queued entries.
    status,     \* none, present or gone (removed from its pool).
    held,       \* The record's standing reference: its table handle or its transit slot.
    everFwd,    \* Records that were ever forwarded.
    dpc,        \* A record's destroy: idle, or walking `close_toward`.
    dcur,       \* The record `close_toward` is at.
    dead,       \* The end is closed, lost in transit, or its world dissolved.
    cur,        \* The end's current record.
    moves,      \* Re-homes the end has made.
    mpc,        \* The end's re-home: idle, frozen, walk, settle, release.
    src,        \* The record being re-homed.
    dst,        \* The world it is re-homed into.
    copied,     \* The link copied under the freeze.
    snap,       \* The inbox copied under the freeze.
    cursor,     \* The record the walk is at.
    settleTo,   \* The link the walk settles its home to.
    waited,     \* Records whose freeze this walk waited out.
    follows,    \* Forwards this walk followed.
    wpc,        \* The end's write: idle or write.
    wentry,     \* The entry being written.
    wtarget,    \* The record the write is at.
    whop,       \* first, or forwarded once followed.
    wlink,      \* The writer's own link as it last read it.
    wwaited,    \* The write waited out a freeze at its target.
    committed,  \* Entries committed into an inbox.
    read,       \* Entries read out.
    destroyed   \* Entries destroyed with their record.

recV == << home, link, frozen, inbox, status, held, everFwd >>
destroyV == << dpc, dcur >>
endV == << dead, cur, moves >>
moverV == << mpc, src, dst, copied, snap, cursor, settleTo, waited, follows >>
writerV == << wpc, wentry, wtarget, whop, wlink, wwaited >>
entryV == << committed, read, destroyed >>
vars == << worldState, recV, destroyV, endV, moverV, writerV, entryV >>

Init ==
    /\ worldState = [w \in Worlds |-> "Live"]
    /\ home = [r \in Rec |-> IF r[2] = 0 THEN 1 ELSE 0]
    /\ link = [r \in Rec |-> CASE r = <<"A", 0>> -> Open(<<"B", 0>>)
                               [] r = <<"B", 0>> -> Open(<<"A", 0>>)
                               [] OTHER -> NoLink]
    /\ frozen = [r \in Rec |-> FALSE]
    /\ inbox = [r \in Rec |-> {}]
    /\ status = [r \in Rec |-> IF r[2] = 0 THEN "present" ELSE "none"]
    /\ held = [r \in Rec |-> r[2] = 0]
    /\ everFwd = {}
    /\ dpc = [r \in Rec |-> "idle"]
    /\ dcur = [r \in Rec |-> NoRec]
    /\ dead = [e \in Ends |-> FALSE]
    /\ cur = [e \in Ends |-> <<e, 0>>]
    /\ moves = [e \in Ends |-> 0]
    /\ mpc = [e \in Ends |-> "idle"]
    /\ src = [e \in Ends |-> NoRec]
    /\ dst = [e \in Ends |-> 0]
    /\ copied = [e \in Ends |-> NoLink]
    /\ snap = [e \in Ends |-> {}]
    /\ cursor = [e \in Ends |-> NoRec]
    /\ settleTo = [e \in Ends |-> NoLink]
    /\ waited = [e \in Ends |-> {}]
    /\ follows = [e \in Ends |-> 0]
    /\ wpc = [e \in Ends |-> "idle"]
    /\ wentry = [e \in Ends |-> NoRec]
    /\ wtarget = [e \in Ends |-> NoRec]
    /\ whop = [e \in Ends |-> "first"]
    /\ wlink = [e \in Ends |-> NoLink]
    /\ wwaited = [e \in Ends |-> FALSE]
    /\ committed = {}
    /\ read = {}
    /\ destroyed = {}

Present(r) == status[r] = "present"

\* `reach` pins the record's world as Live, then `acquire` takes a count. A count of zero refuses,
\* except that a forwarded record outlives every link that names it.
Reachable(r) ==
    /\ Present(r)
    /\ worldState[home[r]] = "Live"
    /\ held[r] \/ (ForwardedOutlives /\ IsFwd(link[r]))

\* What the walk does at record r. A forwarded record is read even in a dying world, and one that is
\* gone was unnamed or torn down, so its walk's home no longer holds the copied link.
Meets(r) ==
    IF ForwardedOutlives
    THEN CASE Present(r) /\ frozen[r] -> "wait"
           [] Present(r) /\ IsFwd(link[r]) -> "follow"
           [] r \in everFwd -> "stop"
           [] ~Reachable(r) -> "close"
           [] IsOpen(link[r]) -> "done"
           [] OTHER -> "close"
    ELSE CASE ~Reachable(r) -> "close"
           [] frozen[r] -> "wait"
           [] IsFwd(link[r]) -> "follow"
           [] IsOpen(link[r]) -> "done"
           [] OTHER -> "close"

\* A read in world w takes the message holding the end and runs `rehome`: under a pin of the old
\* world it freezes the record and copies its inbox and link.
Begin(e, w) ==
    LET s == cur[e] IN
    /\ mpc[e] = "idle" /\ wpc[e] = "idle" /\ ~dead[e]
    /\ moves[e] < Moves(e)
    /\ w # home[s]
    /\ worldState[w] = "Live"
    /\ worldState[home[s]] = "Live"
    /\ frozen' = [frozen EXCEPT ![s] = TRUE]
    /\ mpc' = [mpc EXCEPT ![e] = "frozen"]
    /\ src' = [src EXCEPT ![e] = s]
    /\ dst' = [dst EXCEPT ![e] = w]
    /\ copied' = [copied EXCEPT ![e] = link[s]]
    /\ snap' = [snap EXCEPT ![e] = inbox[s]]
    /\ moves' = [moves EXCEPT ![e] = @ + 1]
    /\ waited' = [waited EXCEPT ![e] = {}]
    /\ follows' = [follows EXCEPT ![e] = 0]
    /\ UNCHANGED << worldState, home, link, inbox, status, held, everFwd, destroyV, dead, cur,
                    cursor, settleTo, writerV, entryV >>

\* The old world is no longer live: `arrive` makes a dead entry, and the record waits for teardown.
Lost(e, w) ==
    LET s == cur[e] IN
    /\ mpc[e] = "idle" /\ wpc[e] = "idle" /\ ~dead[e]
    /\ moves[e] < Moves(e)
    /\ w # home[s]
    /\ worldState[w] = "Live"
    /\ worldState[home[s]] = "Dying"
    /\ dead' = [dead EXCEPT ![e] = TRUE]
    /\ UNCHANGED << worldState, recV, destroyV, cur, moves, moverV, writerV, entryV >>

\* `Endpoint::place` with the copy, then under the old record's lock: unfreeze, empty its inbox,
\* note a close recorded during the freeze, and forward it if its link is still open.
Forward(e) ==
    LET s == src[e]
        m == <<e, moves[e]>>
        forwards == IsOpen(link[s])
        closedMeanwhile == link[s] # copied[e]
    IN
    /\ mpc[e] = "frozen"
    /\ home' = [home EXCEPT ![m] = dst[e]]
    /\ link' = [link EXCEPT ![m] = copied[e], ![s] = IF forwards THEN Fwd(m) ELSE @]
    /\ frozen' = [frozen EXCEPT ![s] = FALSE]
    /\ inbox' = [inbox EXCEPT ![m] = snap[e], ![s] = {}]
    /\ status' = [status EXCEPT ![m] = "present"]
    /\ held' = [held EXCEPT ![m] = TRUE]
    /\ everFwd' = IF forwards THEN everFwd \cup {s} ELSE everFwd
    /\ cur' = [cur EXCEPT ![e] = m]
    /\ \/ /\ IsOpen(copied[e]) /\ closedMeanwhile
          /\ mpc' = [mpc EXCEPT ![e] = "settle"]
          /\ settleTo' = [settleTo EXCEPT ![e] = Closed]
          /\ UNCHANGED cursor
       \/ /\ IsOpen(copied[e]) /\ ~closedMeanwhile
          /\ mpc' = [mpc EXCEPT ![e] = "walk"]
          /\ cursor' = [cursor EXCEPT ![e] = Target(copied[e])]
          /\ UNCHANGED settleTo
       \/ /\ ~IsOpen(copied[e])
          /\ mpc' = [mpc EXCEPT ![e] = "release"]
          /\ UNCHANGED << cursor, settleTo >>
    /\ UNCHANGED << worldState, destroyV, dead, moves, src, dst, copied, snap, waited, follows,
                    writerV, entryV >>

WalkFixed == UNCHANGED << worldState, home, frozen, inbox, status, held, everFwd, destroyV, endV,
                          src, dst, copied, snap, writerV, entryV >>

\* `repoint`: each step reaches one record under its lock and lets it go.
WalkWait(e) ==
    /\ mpc[e] = "walk"
    /\ Meets(cursor[e]) = "wait"
    /\ waited' = [waited EXCEPT ![e] = @ \cup {cursor[e]}]
    /\ UNCHANGED << link, mpc, cursor, settleTo, follows >>
    /\ WalkFixed

WalkFollow(e) ==
    /\ mpc[e] = "walk"
    /\ Meets(cursor[e]) = "follow"
    /\ ~Recheck \/ link[cur[e]] = copied[e]
    /\ cursor' = [cursor EXCEPT ![e] = Target(link[cursor[e]])]
    /\ follows' = [follows EXCEPT ![e] = @ + 1]
    /\ UNCHANGED << link, mpc, settleTo, waited >>
    /\ WalkFixed

\* A peer's re-home that finished meanwhile already linked both ends.
WalkStop(e) ==
    /\ mpc[e] = "walk"
    /\ \/ Meets(cursor[e]) = "stop"
       \/ Meets(cursor[e]) = "follow" /\ Recheck /\ link[cur[e]] # copied[e]
    /\ mpc' = [mpc EXCEPT ![e] = "release"]
    /\ UNCHANGED << link, cursor, settleTo, waited, follows >>
    /\ WalkFixed

WalkDone(e) ==
    /\ mpc[e] = "walk"
    /\ Meets(cursor[e]) = "done"
    /\ link' = [link EXCEPT ![cursor[e]] = Open(cur[e])]
    /\ mpc' = [mpc EXCEPT ![e] = "settle"]
    /\ settleTo' = [settleTo EXCEPT ![e] = Open(cursor[e])]
    /\ UNCHANGED << cursor, waited, follows >>
    /\ WalkFixed

WalkClose(e) ==
    /\ mpc[e] = "walk"
    /\ Meets(cursor[e]) = "close"
    /\ mpc' = [mpc EXCEPT ![e] = "settle"]
    /\ settleTo' = [settleTo EXCEPT ![e] = Closed]
    /\ UNCHANGED << link, cursor, waited, follows >>
    /\ WalkFixed

Walk(e) == WalkWait(e) \/ WalkFollow(e) \/ WalkStop(e) \/ WalkDone(e) \/ WalkClose(e)

\* `settle_home`: a separate lock section after the walk let the peer go.
Settle(e) ==
    /\ mpc[e] = "settle"
    /\ link' = IF link[cur[e]] = copied[e]
               THEN [link EXCEPT ![cur[e]] = settleTo[e]]
               ELSE link
    /\ mpc' = [mpc EXCEPT ![e] = "release"]
    /\ UNCHANGED << worldState, home, frozen, inbox, status, held, everFwd, destroyV, endV, src,
                    dst, copied, snap, cursor, settleTo, waited, follows, writerV, entryV >>

\* The transit reference goes back to the old world; the new home is then in a table.
Release(e) ==
    /\ mpc[e] = "release"
    /\ held' = [held EXCEPT ![src[e]] = FALSE]
    /\ mpc' = [mpc EXCEPT ![e] = "idle"]
    /\ UNCHANGED << worldState, home, link, frozen, inbox, status, everFwd, destroyV, endV, src,
                    dst, copied, snap, cursor, settleTo, waited, follows, writerV, entryV >>

\* `write` reads its own link; a refusal there changes nothing.
WBegin(e, x) ==
    LET me == cur[e] IN
    /\ wpc[e] = "idle" /\ mpc[e] = "idle" /\ ~dead[e]
    /\ worldState[home[me]] = "Live"
    /\ x[1] = e /\ x \notin committed
    /\ IsOpen(link[me])
    /\ wpc' = [wpc EXCEPT ![e] = "write"]
    /\ wentry' = [wentry EXCEPT ![e] = x]
    /\ wtarget' = [wtarget EXCEPT ![e] = Target(link[me])]
    /\ whop' = [whop EXCEPT ![e] = "first"]
    /\ wlink' = [wlink EXCEPT ![e] = link[me]]
    /\ wwaited' = [wwaited EXCEPT ![e] = FALSE]
    /\ UNCHANGED << worldState, recV, destroyV, endV, moverV, entryV >>

WFixed == UNCHANGED << worldState, home, link, frozen, status, held, everFwd, destroyV, endV,
                       moverV, read, destroyed >>

WaitsHere(e) == frozen[wtarget[e]] /\ WriteWaitsFreeze

Resends(e) ==
    /\ whop[e] = "first"
    /\ link[cur[e]] # wlink[e]
    /\ IsOpen(link[cur[e]])

Follows(e) ==
    /\ IsFwd(link[wtarget[e]])
    /\ whop[e] = "first" \/ wwaited[e]

WResend(e) ==
    /\ wpc[e] = "write"
    /\ ~Reachable(wtarget[e])
    /\ Resends(e)
    /\ wtarget' = [wtarget EXCEPT ![e] = Target(link[cur[e]])]
    /\ wlink' = [wlink EXCEPT ![e] = link[cur[e]]]
    /\ wwaited' = [wwaited EXCEPT ![e] = FALSE]
    /\ UNCHANGED << wpc, wentry, whop, inbox, committed >>
    /\ WFixed

WWait(e) ==
    /\ wpc[e] = "write"
    /\ Reachable(wtarget[e])
    /\ WaitsHere(e)
    /\ wwaited' = [wwaited EXCEPT ![e] = TRUE]
    /\ UNCHANGED << wpc, wentry, wtarget, whop, wlink, inbox, committed >>
    /\ WFixed

WCommit(e) ==
    /\ wpc[e] = "write"
    /\ Reachable(wtarget[e])
    /\ ~WaitsHere(e)
    /\ link[wtarget[e]] = Open(cur[e])
    /\ inbox' = [inbox EXCEPT ![wtarget[e]] = @ \cup {wentry[e]}]
    /\ committed' = committed \cup {wentry[e]}
    /\ wpc' = [wpc EXCEPT ![e] = "idle"]
    /\ UNCHANGED << wentry, wtarget, whop, wlink, wwaited >>
    /\ WFixed

\* After a waited-out freeze the forward it left is followed whatever the hop.
WFollow(e) ==
    /\ wpc[e] = "write"
    /\ Reachable(wtarget[e])
    /\ ~WaitsHere(e)
    /\ Follows(e)
    /\ wtarget' = [wtarget EXCEPT ![e] = Target(link[wtarget[e]])]
    /\ whop' = [whop EXCEPT ![e] = "forwarded"]
    /\ wwaited' = [wwaited EXCEPT ![e] = FALSE]
    /\ UNCHANGED << wpc, wentry, wlink, inbox, committed >>
    /\ WFixed

WRefuse(e) ==
    /\ wpc[e] = "write"
    /\ \/ ~Reachable(wtarget[e]) /\ ~Resends(e)
       \/ /\ Reachable(wtarget[e])
          /\ ~WaitsHere(e)
          /\ link[wtarget[e]] # Open(cur[e])
          /\ ~Follows(e)
    /\ wpc' = [wpc EXCEPT ![e] = "idle"]
    /\ UNCHANGED << wentry, wtarget, whop, wlink, wwaited, inbox, committed >>
    /\ WFixed

WStep(e) == WResend(e) \/ WWait(e) \/ WCommit(e) \/ WFollow(e) \/ WRefuse(e)

Read(e, x) ==
    /\ mpc[e] = "idle" /\ ~dead[e]
    /\ worldState[home[cur[e]]] = "Live"
    /\ x \in inbox[cur[e]]
    /\ inbox' = [inbox EXCEPT ![cur[e]] = @ \ {x}]
    /\ read' = read \cup {x}
    /\ UNCHANGED << worldState, home, link, frozen, status, held, everFwd, destroyV, endV, moverV,
                    writerV, committed, destroyed >>

\* Closing the handle drops the last standing reference.
Close(e) ==
    /\ mpc[e] = "idle" /\ wpc[e] = "idle" /\ ~dead[e]
    /\ worldState[home[cur[e]]] = "Live"
    /\ dead' = [dead EXCEPT ![e] = TRUE]
    /\ held' = [held EXCEPT ![cur[e]] = FALSE]
    /\ UNCHANGED << worldState, home, link, frozen, inbox, status, everFwd, destroyV, cur, moves,
                    moverV, writerV, entryV >>

Kill(w) ==
    /\ worldState[w] = "Live"
    /\ worldState' = [worldState EXCEPT ![w] = "Dying"]
    /\ UNCHANGED << recV, destroyV, endV, moverV, writerV, entryV >>

\* `dissolve` once no pin of the world is held and its own threads left the kernel: a freeze
\* holds a pin of the old world, and a re-home or a write runs on a thread of its world.
Dissolve(w) ==
    /\ worldState[w] = "Dying"
    /\ \A e \in Ends:
         /\ ~(mpc[e] = "frozen" /\ home[src[e]] = w)
         /\ ~(mpc[e] # "idle" /\ dst[e] = w)
         /\ ~(wpc[e] # "idle" /\ home[cur[e]] = w)
    /\ worldState' = [worldState EXCEPT ![w] = "Dead"]
    /\ dead' = [e \in Ends |-> dead[e] \/ home[cur[e]] = w]
    /\ held' = [r \in Rec |-> held[r] /\ home[r] # w]
    /\ UNCHANGED << home, link, frozen, inbox, status, everFwd, destroyV, cur, moves, moverV,
                    writerV, entryV >>

\* `destroy`: drain the inbox, take the link, and close toward the peer it named.
Destroy(r) ==
    /\ Present(r) /\ ~held[r] /\ dpc[r] = "idle"
    /\ ~(ForwardedOutlives /\ IsFwd(link[r]))
    /\ destroyed' = destroyed \cup inbox[r]
    /\ inbox' = [inbox EXCEPT ![r] = {}]
    /\ link' = [link EXCEPT ![r] = Closed]
    /\ IF IsOpen(link[r])
       THEN /\ dpc' = [dpc EXCEPT ![r] = "toward"]
            /\ dcur' = [dcur EXCEPT ![r] = Target(link[r])]
            /\ UNCHANGED status
       ELSE /\ status' = [status EXCEPT ![r] = "gone"]
            /\ UNCHANGED destroyV
    /\ UNCHANGED << worldState, home, frozen, held, everFwd, endV, moverV, writerV, committed,
                    read >>

\* `close_toward`, one record per step, following forwards.
Toward(r) ==
    LET p == dcur[r] IN
    /\ dpc[r] = "toward"
    /\ IF Reachable(p) /\ IsFwd(link[p])
       THEN /\ dcur' = [dcur EXCEPT ![r] = Target(link[p])]
            /\ UNCHANGED << link, dpc, status >>
       ELSE /\ link' = IF Reachable(p) /\ link[p] = Open(r)
                       THEN [link EXCEPT ![p] = Closed]
                       ELSE link
            /\ dpc' = [dpc EXCEPT ![r] = "idle"]
            /\ status' = [status EXCEPT ![r] = "gone"]
            /\ UNCHANGED dcur
    /\ UNCHANGED << worldState, home, frozen, inbox, held, everFwd, endV, moverV, writerV,
                    entryV >>

\* A link, or a walk, close or write that follows one, names the record.
Named(r) ==
    \/ \E x \in Rec: Present(x) /\ link[x] \in {Open(r), Fwd(r)}
    \/ \E x \in Rec: dpc[x] = "toward" /\ dcur[x] = r
    \/ \E e \in Ends: mpc[e] = "walk" /\ cursor[e] = r
    \/ \E e \in Ends: wpc[e] = "write" /\ wtarget[e] = r

\* A forwarded record in a live world goes once nothing names it.
Retire(r) ==
    /\ ForwardedOutlives
    /\ Present(r) /\ ~held[r] /\ IsFwd(link[r])
    /\ worldState[home[r]] = "Live"
    /\ ~Named(r)
    /\ status' = [status EXCEPT ![r] = "gone"]
    /\ destroyed' = destroyed \cup inbox[r]
    /\ inbox' = [inbox EXCEPT ![r] = {}]
    /\ UNCHANGED << worldState, home, link, frozen, held, everFwd, destroyV, endV, moverV,
                    writerV, committed, read >>

\* A dead world's teardown completes the repoint of each forwarded record: every name of it
\* becomes a name of its forward, once no frozen record holds such a name.
Teardown(r) ==
    LET t == Target(link[r]) IN
    /\ ForwardedOutlives
    /\ Present(r) /\ ~held[r] /\ IsFwd(link[r])
    /\ worldState[home[r]] = "Dead"
    /\ \A x \in Rec: frozen[x] => link[x] \notin {Open(r), Fwd(r)}
    /\ link' = [x \in Rec |-> CASE link[x] = Open(r) -> Open(t)
                                [] link[x] = Fwd(r) -> Fwd(t)
                                [] OTHER -> link[x]]
    /\ status' = [status EXCEPT ![r] = "gone"]
    /\ destroyed' = destroyed \cup inbox[r]
    /\ inbox' = [inbox EXCEPT ![r] = {}]
    /\ UNCHANGED << worldState, home, frozen, held, everFwd, destroyV, endV, moverV, writerV,
                    committed, read >>

Next ==
    \/ \E e \in Ends, w \in Worlds: Begin(e, w) \/ Lost(e, w)
    \/ \E e \in Ends: Forward(e) \/ Walk(e) \/ Settle(e) \/ Release(e) \/ WStep(e) \/ Close(e)
    \/ \E e \in Ends, x \in Entries: WBegin(e, x) \/ Read(e, x)
    \/ \E w \in Worlds: Kill(w) \/ Dissolve(w)
    \/ \E r \in Rec: Destroy(r) \/ Toward(r) \/ Retire(r) \/ Teardown(r)

\* The adversary starts re-homes, writes, reads, closes and kills; every started step finishes.
Fairness ==
    /\ \A e \in Ends:
         /\ WF_vars(Forward(e)) /\ WF_vars(Walk(e)) /\ WF_vars(Settle(e))
         /\ WF_vars(Release(e)) /\ WF_vars(WStep(e))
    /\ \A w \in Worlds: WF_vars(Dissolve(w))
    /\ \A r \in Rec:
         WF_vars(Destroy(r)) /\ WF_vars(Toward(r)) /\ WF_vars(Retire(r)) /\ WF_vars(Teardown(r))

Spec == Init /\ [][Next]_vars /\ Fairness

\* No re-home, write, close or teardown is in progress.
Quiescent ==
    /\ \A e \in Ends: mpc[e] = "idle" /\ wpc[e] = "idle"
    /\ \A w \in Worlds: worldState[w] # "Dying"
    /\ \A r \in Rec:
         Present(r) /\ ~held[r] =>
             ForwardedOutlives /\ IsFwd(link[r]) /\ worldState[home[r]] = "Live"

\* Link: at rest each live end names its peer's current record, or reports peer closed and its
\* peer is dead. So no live end names a destroyed record, or reports a live peer closed.
Link ==
    Quiescent =>
        \A e \in Ends:
            ~dead[e] =>
                link[cur[e]] = IF dead[Other(e)] THEN Closed ELSE Open(cur[Other(e)])

\* PeerClosed: once one end is dead, the other, wherever it moves, comes to report peer closed.
PeerClosed ==
    \A e \in Ends:
        dead[e] ~> (dead[Other(e)] \/ (mpc[Other(e)] = "idle" /\ link[cur[Other(e)]] = Closed))

\* Conservation: a committed entry is in exactly one inbox, read once, or destroyed with its
\* record; an entry never committed is nowhere.
Places(x) ==
    Cardinality({r \in Rec: x \in inbox[r]})
        + (IF x \in read THEN 1 ELSE 0)
        + (IF x \in destroyed THEN 1 ELSE 0)

Conservation == \A x \in Entries: Places(x) = IF x \in committed THEN 1 ELSE 0

\* NoMutualWait: every wait is on a freeze, and a freeze is held only by a re-home between its
\* freeze and its forward, a step that waits on nothing.
NoMutualWait == \A r \in Rec: frozen[r] => mpc[r[1]] = "frozen" /\ src[r[1]] = r

EveryRehomeEnds == \A e \in Ends: mpc[e] # "idle" ~> mpc[e] = "idle"

EveryWriteEnds == \A e \in Ends: wpc[e] = "write" ~> wpc[e] = "idle"

\* Bound: one walk waits out at most WaitBound freezes and follows at most FollowBound forwards.
Bound == \A e \in Ends: Cardinality(waited[e]) <= WaitBound /\ follows[e] <= FollowBound

TypeOK ==
    /\ worldState \in [Worlds -> {"Live", "Dying", "Dead"}]
    /\ home \in [Rec -> 0..WorldCount]
    /\ link \in [Rec -> Links]
    /\ frozen \in [Rec -> BOOLEAN]
    /\ held \in [Rec -> BOOLEAN]
    /\ inbox \in [Rec -> SUBSET Entries]
    /\ status \in [Rec -> {"none", "present", "gone"}]
    /\ dpc \in [Rec -> {"idle", "toward"}]
    /\ mpc \in [Ends -> {"idle", "frozen", "walk", "settle", "release"}]
    /\ wpc \in [Ends -> {"idle", "write"}]
    /\ \A e \in Ends: cur[e] \in Rec /\ moves[e] \in 0..Moves(e)
    /\ committed \subseteq Entries /\ read \subseteq Entries /\ destroyed \subseteq Entries

=============================================================================
