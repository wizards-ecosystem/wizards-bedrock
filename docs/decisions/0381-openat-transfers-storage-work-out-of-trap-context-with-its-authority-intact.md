# D-381 OpenAt transfers storage work out of trap context with its authority intact

Status: Accepted, 2026-10-10. Amends D-163, D-173, D-192 and D-200; closes PW-590.

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
recognizes the settling entry as a retry, not a stale tombstone, and leaves the
payload queued until cancellation restores the pending owner. Once running,
the operation is not cancellable and retains the ring and reservation until
worker settlement succeeds.

A host source contract finds no `vfs::resolve` in `Queue::open_at`, exactly one
`FILE_WORK.submit` there and the one OpenAt resolution in thread-context
`service`. A machine witness pauses the worker, cancels one pending operation
with its promised handle, observes two distinct completions and requires the
work queue and promise to be empty before quiet; a second round injects
terminal-post refusal, keeps the request and promise owned, resumes the worker
and requires one successful handle completion with no duplicate or leak.
Mutation 1,771 restores an inline resolution and fails that contract, and
mutation 1,772 leaves the cancelled payload queued; on 2026-10-10 it failed only
`a cancelled open releases its reserved handle before quiet`.

**Rationale.** Codex Security occurrence `occ_58f0b32a48b6c444cfe0d501` found
that `Call::RingEnter` synchronously reached `vfs::resolve`, where a FAT
resolution can mount the volume and take `CONTROLLER`'s `ThreadLock`, which
refuses from trap context because blocking there would carry the handler's
stack. The implemented directory-capability route therefore made a valid OpenAt
a kernel-halting path. Making `ThreadLock` legal in trap context would break the
scheduler and stack-lifetime boundary for every caller; special-casing
`nvme::has_controller` would leave mount and later filesystem work able to
block; reserving the handle only in the worker would turn an admitted operation
into a late quota failure and undo D-163; and a per-request task would make
hostile admission unbounded. The existing fixed worker already supplies bounded
pressure, thread context, cancellation and retryable completion.
