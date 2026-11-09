# D-236 Restoring a target dependency invalidates everything linked against it

Status: Accepted, 2026-08-29. Amends D-233; advances PW-001.

**Context.** The 283-entry successor on clean commit
`095b624bd1601cb3e669b5ade7c614bc5e224405` made 21 outcomes durable before
being stopped during mutation 22. Nineteen of the first twenty outcomes were
exact and none survived, but mutation 14 remained inconclusive twice. Its
serial transcript said user threads were running on tables that were not their
own: mutation 12's `wost` thread-switch defect was still present in the linked
kernel even though the isolated source had been restored and mutation 13 had
completed. Mutation 13's earlier terminal user-copy fault had hidden that
contamination from its own exact outcome.

D-233 invalidated only the package whose source was restored. That removes a
restored host artifact, but the package-only command names no target and
reported that it removed zero files after a cross-target kernel build. Cargo
therefore retained both the `x86_64-unknown-none` dependency artifact and the
already-linked `wizard-kernel` artifact and incremental state which consumed
it. A later kernel mutation can be evaluated alongside an older dependency
mutation. Cleaning the dependency alone would not have been enough either:
restored bytes rebuild to their old fingerprint, so the consumer stays fresh
with the defect already inlined into it.

**Decision.** Every target-side invalidation names the fixed
`x86_64-unknown-none` target and every target artifact that already linked the
restored one. Restoring `wost` invalidates both it and `wizard-kernel`.
Restoring a `libs/` input invalidates that library, `wost` and `wizard-kernel`
for the target, and every host member of the workspace for the host. A library
is inlined into whatever links it, so naming only the far end of a chain
leaves the near one holding the defect: `wost` carries libraries into the
kernel, and `xtask` links three of them itself. The host list is read from the
manifest's `default-members` rather than repeated here, and a restored library
missing from it is an error rather than a silent partial clean. Kernel inputs
invalidate the target kernel; `xtask` inputs invalidate the host package,
which nothing else links. The invalidation remains package-scoped rather than
deleting the private target: third-party dependencies keep their cheap
incremental path, while every later witness must rebuild from restored
first-party sources.

**Alternatives.** Trusting Cargo's timestamp and incremental dependency graph
was rejected because the clean successor reproduced linked behavior absent
from the restored bytes. Cleaning the entire private target after every
mutation was rejected because it would repeatedly rebuild unrelated host
dependencies and turn a bounded one-hour qualification into a much longer
one. Crediting mutation 14's many red checks was rejected because none was its
registered terminal cause.

**Consequences.** Two regression fixtures build against a mutated dependency
and require every artifact that consumed it to be absent after the restore.
The first cross-builds `wizard-kernel` against no-std `wost`. The second adds
an inlined `libs/` crate beneath both `wost` and a host consumer standing in
for `xtask`, and requires all five artifacts to go. Disabling reverse-consumer
invalidation leaves the kernel artifact present, cleaning the host target
leaves both cross-target artifacts present, stopping the target chain at
`wizard-kernel` leaves the `wost` artifact that inlined the library present,
and naming only the restored library on the host leaves the host artifact that
inlined it present. Mutations 284 through 287 retain those four defects. The
interrupted 283-entry run is diagnostic only. PW-001 belongs to a clean
287-entry successor. No target code, unsafe operation, wire ABI or
machine-check count changed.

**Superseded by D-237 (2026-08-29).** The reverse consumers named here are
real, and so is the evidence that found them; naming them one by one is what
D-237 removes. Restoring a source forward in time makes Cargo invalidate every
one of them, including the ones this decision would still have missed.
