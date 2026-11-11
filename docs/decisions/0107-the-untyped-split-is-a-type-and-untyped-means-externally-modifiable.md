# D-107 The untyped split is a type, and "untyped" means externally modifiable

Status: Accepted, 2026-08-24.

**Decision.** D-44's untyped/typed split is a separate type, `mm::USegment`,
with two doors instead of one flag. `map_owned` **refuses** `Flags::USER`
outright; `map_user` takes a `USegment` and adds `USER` itself. Converting a
`Segment` into a `USegment` marks the caller giving the contents away, and is
the only way to the user door.

The only road back is `USegment::reclaim`, which **zeroes first**. A frame
handed back still holding a program's stack is how one process reads another's
memory. The zeroing is the conversion itself: D-12's zero-on-allocate, applied
at the other end.

"Untyped" here is *externally modifiable, plain bytes only*. It is **not**
seL4's reading, memory donated to the kernel to be retyped into an object; D-44
already refused that, and the crate doc pins the word where the type is.

**Rationale.** A flag on `Segment` can be forgotten at a call site, while the
split makes the compiler ask. Adding `USER` inside `map_user`, not accepting it
from the caller, closes the matching hole in the other direction.

**Consequences.** `Segment` has `read_at`, the counterpart to `write_at`, and
only on `Segment`: reading a `USegment` into the kernel without clearing first
is what `reclaim` exists to prevent. `kernel_memory_cannot_be_published_to_ring_three`
checks what is *in* a reclaimed frame, not whether it is reachable, by D-105's
second rule.

Left owed: per-process quota accounting for these frames, D-44's other half,
which belongs with the handle table's.
