# D-107 The untyped split is a type, and "untyped" means externally modifiable

Status: Accepted, 2026-08-24.

**Context.** D-44 adopted the untyped/typed split as a *type discipline* and
rejected it as an accounting mechanism. `wost/src/mm/mod.rs` has said since
Phase 3 that the split was "absent on purpose … it arrives with the first
caller that can map a frame into a second address space". D-106 built that
caller.

**Options.** Track it beside the frame in the address space; carry a flag on
`Segment`; or make it a separate type.

**Decision.** A separate type, `mm::USegment`, and two doors rather than one
flag. `map_owned` **refuses** `Flags::USER` outright; `map_user` takes a
`USegment` and adds `USER` itself. Converting a `Segment` into a `USegment` is
the caller noticing that it is giving the contents away, and it is the only way
to reach the user door.

The road back is `USegment::reclaim`, which **zeroes first** and is the only
road: a frame that held a program's stack, handed to the kernel still holding
it, is how one process reads another's memory. The zeroing is the conversion
rather than a courtesy attached to it — D-12's zero-on-allocate, applied at the
other end.

**Rationale for the meaning.** "Untyped" here is *externally modifiable, plain
bytes only*. It is deliberately **not** seL4's reading — memory donated to the
kernel to be retyped into an object — because this kernel does not build
objects out of donated memory, and D-44 already refused that. Two systems using
one word for two ideas is how a discipline becomes a vocabulary, so the word is
pinned in the crate doc where the type is.

A flag on `Segment` would have been smaller and would have carried none of
this: a flag can be forgotten at a call site, and the whole value of the split
is that the compiler asks. Adding `USER` inside `map_user` rather than
accepting it from the caller closes the matching hole in the other direction.

**Consequences.** `Segment` gained `read_at`, the counterpart to `write_at`,
and deliberately only on `Segment`: reading a `USegment` into the kernel is
what `reclaim` exists to make impossible without clearing first. The loader,
the stack placer and two checks now go through `map_user`.

`kernel_memory_cannot_be_published_to_ring_three` is in the gate at 111 checks,
and it was broken twice. The second break is the one worth recording: removing
the zeroing from `reclaim` left the first version of the check green, because
it asked whether the frame was still *reachable* rather than what was *in* it.
That is D-105's second rule — a check must fail for the reason it names — and
it took writing the break to see it. `read_at` exists because fixing it needed
a way to look.

Left owed: per-process quota accounting for these frames, which is D-44's other
half and belongs with the handle table's.
