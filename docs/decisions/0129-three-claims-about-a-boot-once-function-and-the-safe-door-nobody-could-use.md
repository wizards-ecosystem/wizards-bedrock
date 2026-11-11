# D-129 Three claims about a boot-once function, and the safe door nobody could use

Status: Accepted, 2026-08-25.

**Decision.** `pic::silence`'s two-caller race is stated, not fixed, and
recorded against D-56. Its recheck closes the window only once the first caller
has *stored*, so a second caller can report `PortsUnavailable` for a PIC about
to be silenced correctly. `silence` runs once, at boot, from one caller, and a
race-free version would be a path nothing on this machine can execute and no
check can reach. The code carries the interleaving, written out, with the
reason it is not fixed. The flag *is* set between the two guards, so it is a
preemption away rather than SMP-only: "unreachable on one CPU" and
"unreachable" are different sentences.

**A failed partial claim strands what it got.** `PortBlock` has no `Drop` and
the registry never releases, so a caller claiming a *set* that gets one claim
leaves it spoken for by nothing. `port`'s module doc states that a caller
wanting two claims must survive getting one; `silence` names it rather than
guarding, because on the one path it runs the ports are free.

**`segment::set_kernel_stack` is deleted**, and `write_kernel_stack` records
why. The switch writes `RSP0` *before* it installs the arriving thread's
tables, and since D-125 `is_installed` walks the running tables, so the
evidence-taking form could never accept the arriving thread's stack. A safe
wrapper its only intended caller cannot use by construction is the speculative
API `005` forbids. `mm::managed_frames` goes too: nothing called it.

**Rationale.** A comment that overclaims is worse than no comment, because the
next reader spends their scepticism elsewhere; this class is found only by
reading the comment against the code. `set_kernel_stack` was a *claim in the
shape of a function*, and D-125's honest `is_installed` is what made it
unusable: a safety improvement closed a door that had looked open.

**Consequences.** No check is added, and that is a decision rather than an
omission: the race needs a second CPU, the partial-claim path needs a port the
PIC does not already own, and the deleted function had no callers to regress.

**Left owed.** The two-caller race, and `LEAVES_HELD`'s move into
`arch::percpu`, both against D-56. Whoever does that pass should treat this
entry as its inbox: they are the same work, and neither is checkable before the
second CPU exists.
