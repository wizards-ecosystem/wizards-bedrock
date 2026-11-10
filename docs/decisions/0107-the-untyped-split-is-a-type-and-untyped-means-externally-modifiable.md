# D-107 The untyped split is a type, and "untyped" means externally modifiable

Status: Accepted, 2026-08-24.

**Context.** D-44 adopted the untyped/typed split as a *type discipline* and
rejected it as an accounting mechanism. Since Phase 3 `wost/src/mm/mod.rs` has
said the split was "absent on purpose … it arrives with the first caller that
can map a frame into a second address space". D-106 built that caller.

**Options.** Track it beside the frame in the address space; carry a flag on
`Segment`; or make it a separate type.

**Decision.** A separate type, `mm::USegment`, and two doors instead of one
flag. `map_owned` **refuses** `Flags::USER` outright; `map_user` takes a
`USegment` and adds `USER` itself. Converting a `Segment` into a `USegment`
marks the caller giving the contents away, and is the only way to the user door.

The only road back is `USegment::reclaim`, which **zeroes first**. A frame
handed back still holding a program's stack is how one process reads another's
memory. The zeroing is the conversion itself: D-12's zero-on-allocate, applied
at the other end.

**Rationale for the meaning.** "Untyped" here is *externally modifiable, plain
bytes only*. It is **not** seL4's reading, memory donated to the kernel to be
retyped into an object. This kernel does not build objects out of donated
memory, and D-44 already refused that. Two systems using one word for two ideas
turn a discipline into a vocabulary, so the crate doc pins the word where the
type is.

A flag on `Segment` would have been smaller and carried none of this: a flag can
be forgotten at a call site, while the split makes the compiler ask. Adding
`USER` inside `map_user`, not accepting it from the caller, closes the matching
hole in the other direction.

**Consequences.** `Segment` gained `read_at`, the counterpart to `write_at`, and
only on `Segment`: reading a `USegment` into the kernel without clearing first
is what `reclaim` exists to prevent. The loader, the stack placer and two checks
now go through `map_user`.

`kernel_memory_cannot_be_published_to_ring_three` is in the gate at 111 checks,
and it was broken twice. Removing the zeroing from `reclaim` left the first
version of the check green, because it asked whether the frame was still
*reachable*, not what was *in* it. That is D-105's second rule, a check must
fail for the reason it names, and only writing the break showed it. `read_at`
exists because the fix needed a way to look.

Left owed: per-process quota accounting for these frames, D-44's other half,
which belongs with the handle table's.
