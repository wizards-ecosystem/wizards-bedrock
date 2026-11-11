# D-236 Restoring a target dependency invalidates everything linked against it

Status: Accepted, 2026-08-29. Amends D-233; advances PW-001.

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

**Rationale.** The 283-entry successor on clean commit
`095b624bd1601cb3e669b5ade7c614bc5e224405` left mutation 14 inconclusive twice
because mutation 12's `wost` defect was still linked into the kernel after its
source was restored. D-233 invalidated only the package whose source was
restored, and that package-only command names no target, so Cargo retained the
`x86_64-unknown-none` dependency artifact and the `wizard-kernel` artifact
which consumed it. Cleaning the dependency alone would not suffice either:
restored bytes rebuild to their old fingerprint, so the consumer stays fresh
with the defect already inlined into it.

**Alternatives.** Trusting Cargo's timestamp and incremental dependency graph
was rejected because the clean successor reproduced linked behavior absent
from the restored bytes. Cleaning the entire private target after every
mutation was rejected because it would repeatedly rebuild unrelated host
dependencies and turn a bounded one-hour qualification into a much longer
one. Crediting mutation 14's many red checks was rejected because none was its
registered terminal cause.

**Consequences.** Two regression fixtures build against a mutated dependency
and require every artifact that consumed it to be absent after the restore,
one cross-building `wizard-kernel` against no-std `wost` and one inlining a
`libs/` crate beneath both `wost` and a host consumer. Mutations 284 through
287 keep them red for each missing reverse consumer. The interrupted 283-entry
run is diagnostic only.

**Superseded by D-237 (2026-08-29).** The reverse consumers named here are
real, and so is the evidence that found them; naming them one by one is what
D-237 removes. Restoring a source forward in time makes Cargo invalidate every
one of them, including the ones this decision would still have missed.
