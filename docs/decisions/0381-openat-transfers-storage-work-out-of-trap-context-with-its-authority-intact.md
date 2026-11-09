# D-381 OpenAt transfers storage work out of trap context with its authority intact

Status: Accepted (2026-10-10), amends D-163, D-173, D-192 and D-200; closes PW-590

**Context.** Codex Security occurrence `occ_58f0b32a48b6c444cfe0d501`
found that `Call::RingEnter` synchronously reached `Queue::open_at`, which
called `vfs::resolve`. A FAT resolution can mount the volume, ask NVMe whether
a controller exists and take `CONTROLLER`'s `ThreadLock`. `ThreadLock` refuses
from trap context because blocking or switching there would carry the
handler's stack. The shipped bootstrap currently grants only a channel, but
the implemented directory-capability route—and any future delegation through
it—made a valid OpenAt a kernel-halting path rather than a denied operation.

**Decision.** OpenAt admission reserves the result handle slot first, copies at
most `MAX_PATH` bytes from the shared data window once, resolves and attenuates
the caller's directory capability, and offers one owned request to the existing
fixed eight-entry file-work queue. It performs no VFS, FAT or NVMe work. The
request carries the bounded path, typed directory, held rights and linear
`SlotReservation`; the service worker alone resolves the node, derives the
file or directory rights, installs through that same reservation and posts the
one terminal completion. Read and OpenAt share a queued/running phase and an
exact `(ring, pending slot, private id, kind)` claim. The worker copies a key,
releases the queue leaf, advances the exact pending entry, then removes the
payload, so it never nests two leaf locks. A queued cancellation that posts
successfully removes the exact payload and drops its reservation before the
ring can become quiet. If terminal posting transiently refuses, the worker
recognizes the settling entry as a retry—not a stale tombstone—and leaves the
payload queued until cancellation restores the pending owner. Once running,
the operation is not cancellable and retains the ring and reservation until
worker settlement succeeds.

**Proof required.** A host source contract must find no `vfs::resolve` in
`Queue::open_at`, exactly one `FILE_WORK.submit` there and the one OpenAt
resolution in thread-context `service`. A live-machine witness must pause the
worker, observe one pending operation and one promised handle, cancel it,
observe two distinct completions and require both the work queue and promise
to be empty before quiet. A second round must inject terminal-post refusal,
observe the request and promise remain owned, resume the worker, and require
one successful handle completion with no duplicate or leak. Retained mutations
must independently restore the inline resolution and omit queued-payload
removal.

**Evidence.** Before the repair, the focused contract failed exactly with
`OpenAt admission reached storage while RingEnter was in trap context`; the
same contract passes after the transfer. Mutation 1,771 restores an inline
resolution and fails that exact contract. Mutation 1,772 leaves the cancelled
payload queued and fails only `a cancelled open releases its reserved handle
before quiet` at 280/281. Restoration passes release and debug QEMU at 281/281.
All 1,772 issued mutation identities have a current single source anchor and
registered check. On the final tree, check, Clippy and docs pass; the compiler
audit resolves callers for all 2,030 public API items; supply-chain and vendored
input verification pass; shipped release/debug and both degraded release boots
settle; and the serial host suite passes 746 tests with 10 intentional ignores.

**Alternatives.** Making `ThreadLock` legal in trap context would violate the
scheduler and stack-lifetime boundary for every caller. Special-casing only
`nvme::has_controller` would leave mount and later filesystem work able to
block under the handler. Reserving the handle only in the worker would turn a
successfully admitted operation into a late quota failure and undo D-163.
Allocating a per-request task would make hostile admission unbounded. The
existing fixed worker is the narrowest owner that already supplies bounded
pressure, thread context, cancellation and retryable completion.
