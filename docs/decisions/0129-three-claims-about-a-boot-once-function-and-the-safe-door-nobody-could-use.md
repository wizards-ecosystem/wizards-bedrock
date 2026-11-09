# D-129 Three claims about a boot-once function, and the safe door nobody could use

Status: Accepted, 2026-08-25.

**Context.** D-126 reordered `pic::silence` so its port claims happen outside
both leaf guards. A peer session auditing the same window read the comment I
left and found it asserting something the code does not do — which is the
class D-124 exists for, introduced by the commit that was fixing that class.

**The comment said the recheck answers the race. It does not.** Interleave two
callers: B reads `CONTROLLERS` as `None` and drops the guard; A claims both
port blocks; B's two claims fail; B takes `CONTROLLERS`' guard and finds it
**still** `None`, because A is queued behind B on that same guard; B falls
through and reports `PortsUnavailable` for a PIC that is about to be silenced
correctly. The recheck closes the window only once A has *stored*, and nothing
makes it have done.

Two further things the audit surfaced around it:

- **A failed partial claim strands what it got.** `PortBlock` has no `Drop` and
  the registry never releases — right for a driver that owns its registers for
  the life of the kernel, and a trap for a caller claiming a *set*.
  `claim_block(PRIMARY).zip(claim_block(SECONDARY))` claims PRIMARY, fails on
  SECONDARY, and leaves PRIMARY spoken for by nothing, so no later caller can
  ever silence the PIC. The `?`-chained version this replaced had the same
  outcome and merely reached it less reliably.
- **`segment::set_kernel_stack` had no callers, and could not have any.** It is
  the evidence-taking form of the `RSP0` write: it takes a `MappedStack` and
  refuses unless `is_installed()`. The context switch writes `RSP0` *before* it
  installs the arriving thread's tables, and since D-125 `is_installed` walks
  the running tables — so the arriving thread's stack is never in the space the
  CPU is walking at the moment `RSP0` is set. The switch calls
  `write_kernel_stack` instead, and always did.

**Options for the race.** Restructure `silence` to be correct under
interleaving — a claim-then-publish with the loser releasing — or state the
window and record it as owed.

**Decision.** State it, and record it against D-56.

`silence` runs once, at boot, from one caller. The interleaving needs two, and
this kernel has one CPU. Making it race-free buys a code path nothing on this
machine can execute and no check can reach, which is the shape D-115 kept
finding: an unreachable branch is a claim nobody has tested. The hazard arrives
with the second CPU, and so does the other half of the same work — D-126 owes
`LEAVES_HELD` a move into `arch::percpu`, for the same reason and on the same
schedule. Both belong in D-56's per-CPU pass.

What the code carries instead is the interleaving, written out, with the reason
it is not fixed. The flag *is* set between the two guards, so it is a
preemption away rather than SMP-only — named, because "unreachable on one CPU"
and "unreachable" are different sentences.

The partial-claim consequence moves to `port`'s module doc, where it belongs:
it is a property of the registry, not of the PIC. `silence` names it rather
than guarding, because on the one path it runs the ports are free.

**`set_kernel_stack` is deleted**, and `write_kernel_stack` records why. A safe
wrapper that its only intended caller cannot use by construction is the
speculative API `005` forbids, and leaving it in suggested a road that was
never open — a reader finding it would reasonably assume the switch had simply
not been updated to use it.

`mm::managed_frames` goes with it, on the plain version of the same rule:
nothing has ever called it, `frame_space::FrameSpace::managed_count` is reached
through the stats struct where it is actually wanted, and a `pub fn` in `wost`
is part of the kernel's interface to its own TCB.

**Rationale.** The first two are one idea: a comment that overclaims is worse
than no comment, because the next reader spends their scepticism elsewhere. The
peer found this by reading the comment against the code rather than by
suspecting the code — which is the only way this class is ever found, and the
reason D-124 put as many of these claims as it could under a test.

Deleting the dead door is the same idea one level up. `set_kernel_stack` is a
*claim in the shape of a function*: it says the kernel has a checked way to set
`RSP0`. It does not, it cannot, and the reason it cannot is interesting enough
to write down — D-125 made `is_installed` honest, and an honest `is_installed`
is exactly what makes this function unusable. A safety improvement closed a
door that had looked open.

**Consequences.**

- `pic::silence` is unchanged in behaviour. Its comment now describes the
  interleaving it does not survive, the permanence of a partial claim, and why
  neither is fixed here.
- `port`'s module doc states that a caller wanting two claims must survive
  getting one. There is one such caller.
- `wost` loses two `pub` items and 155 becomes 153. Neither had a caller;
  neither is replaced.
- No check is added, and that is a decision rather than an omission. The race
  needs a second CPU; the partial-claim path needs a port the PIC does not
  already own; the deleted function had no callers to regress. What the gate
  *does* now hold is that `wost`'s unsafe count is pinned (D-124), so removing
  code cannot quietly change it — `MEASURED` is unaffected because neither
  deletion contained any.

**Left owed.** The two-caller race, and `LEAVES_HELD`'s move into
`arch::percpu`, both against D-56. Whoever does that pass should treat this
entry as its inbox: they are the same work, and neither is checkable before the
second CPU exists.
