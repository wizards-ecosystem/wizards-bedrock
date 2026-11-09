# D-229 The ring model carries visibility through every terminal owner

Status: Accepted, 2026-08-29. Closes PW-404.

**Context.** D-112's TLA+ model proves admission accounting over anonymous
counts. D-221's four-event host litmus proves that one release tail and one
acquire read exclude a published index beside an old payload. Neither proof
carries that visibility edge through the operation whose reservation may end
normally, by cancellation or during process retirement. Shuttle cannot fill
the gap: its scheduler is sequentially consistent, which D-112 already proved
by watching a deliberately Relaxed model stay green.

**Decision.** `models/RingMemory.tla` is the finite, identity-bearing model for
that composition. Two distinct operations each reuse one message state first
for a submission and then for its completion. Payload issue, payload
visibility, tail issue, tail visibility, tail acquisition and payload read are
separate transitions. Release permits tail visibility only after payload
visibility; acquire permits the payload read only after the tail observation.
The same transitions therefore cover both queue directions rather than
crediting a one-way publication proof twice.

Admission occurs only after the submitted payload was acquired and takes its
reservation in the same transition which makes the operation live. Normal
completion, cancellation and retirement are three distinct settling causes.
All retain the live identity until terminal publication; that transition
spends exactly its reservation, records one report and changes the same message
state to completion publication. Reaping happens only after the completion
payload is acquired. The checked invariants require no stale payload read,
one reservation per live operation, bounded completion ownership, at most one
terminal report per identity, no vanished admitted operation and explicit
reachability of all three causes.

The correct configuration is not trusted by itself. Four adjacent mutant
configurations independently remove release ordering, acquire ordering,
reservation at admission or the one-shot terminal guard. `cargo xtask
model-check` requires the correct model to finish clean and requires every
mutant to violate its exact named invariant; a mutant that becomes clean or
fails for another reason is a failed qualification. Cancellation and
retirement each have a distinct model state and are required reachable, so
TLC cannot collapse their transitions into the normal path.

The model is also bound back to source at the ordinary host gate. The existing
D-221 checks pin the real atomic and volatile publication operations. New
source checks require `Queue::take` to reserve before consuming or publishing
the submission head, require the settling flag to be claimed under its one
lock, require terminal publication before a pending slot is removed, and
require cancellation and retirement to use that same settlement path. Real
production mutations remove reservation and the one-shot settling guard; the
older release/acquire mutations remain the source-side reordered-memory
witnesses.

**Alternatives.** Enlarging `Ring.tla` was rejected because its anonymous
counts are the clearest proof of admission capacity, while message visibility
and terminal identity would obscure that small model and multiply its state
space. Another Shuttle test was rejected by the measured false green above.
A stress test on x86 was rejected because TSO cannot exhibit every ordering a
portable Wizzy runtime must express. A prose mapping from the model to source
was rejected because it can drift while both code and model remain green.

**Consequences.** Microsoft OpenJDK 21.0.5 and TLC 2026.08.21.155922 completed
the original accounting model at 2,901 generated / 1,040 distinct states and
depth 49, then completed the new model at 2,809 generated / 1,296 distinct
states and depth 31. The four relaxed-store, relaxed-load, no-reservation and
duplicate-terminal configurations violated `NoStalePayload`, `NoStalePayload`,
`ReservationsAreLive` and `OneCompletionPerOperation` respectively. The task
rejects a mutant which becomes clean or names a different invariant.

Removing the real reservation increment, the real one-shot settling guard and
retirement from the model's reachable actions each reddened only its new
source/model-bound host check before restoration; they persist as mutations
262 through 264. D-221's release/acquire mutations 227 and 228 remain the
production memory-order breaks. The model check stays an exceptional Java
qualification, while model inventory, complete invariant registration, exact
mutant registration, cause reachability and both source bindings run in every
CI. A missing checker now points to its actual vendoring instructions, with
that recovery path pinned by mutation 265. No target code, wire ABI, unsafe
operation or machine check changed.
