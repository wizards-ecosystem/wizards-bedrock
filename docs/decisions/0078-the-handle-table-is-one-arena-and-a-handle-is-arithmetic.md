# D-78 The handle table is one arena, and a handle is arithmetic

Status: Accepted, 2026-08-24.

**Context.** `002-architecture.md` owed "handle table representation and
rights taxonomy" before Phase 5. `010` §3 answered the taxonomy: 16 to 24
rights in one `u32`, each naming a distinction the kernel makes about the
object, not a syscall. The layout and lifetime rules remained, and the first
caller freezes them, so they come before the first object.

**Decision.** One global arena, not per-process arrays. A slot's base value is
`generation ‖ arena_index`; a process sees `(base_value << 2 | 0b11) ^
per_table_mixer`. Lookup is arithmetic and one ownership comparison. The
encode/decode arithmetic goes to `libs/handle-space`; the kernel keeps the
arena and the lock.

- **No lowest-free promise.** The reserved low bits forbid a dense encoding,
  so the promise cannot be made.
- **`HandleTable { table: AtomicPtr<HandleArray> }`**: Linux's
  `files_struct`/`fdtable` split, so readers can go lock-free later without
  touching a call site.
- **The table is a refcounted object**, not an inline field of `Process`.
- **Rights are 16 to 24 bits of one `u32`**, universal (`DUPLICATE`,
  `TRANSFER`, `WAIT`, `INSPECT`) split from per-type.
- **Type encodings have headroom for 64 object types.** Policy is monotonic
  and frozen (tighten-only, immutable once a process has children); adding a
  type never renumbers an existing one.
- **Refcounts use `inc_not_zero` from day one.** Close promises no global
  visibility on return.
- **No revocation.**
- The per-process **quota** D-44 assigned to this doc is answered here: a
  process's kernel memory is bounded by what its handle table can name.

**Rationale.** The encoding is Zircon's. Two independent research angles
reached it separately: security, because generation tagging kills
use-after-free on handle reuse, and scalability, via the Scalable
Commutativity Rule. Zircon's contract commutes *as specified*. POSIX's "lowest
free descriptor" is an ordering promise that serialises allocation. sv6
measured the unordered alternative scaling linearly with **no single-core
penalty**: "at one core, both cases perform identically." The scalable choice
is free today, so it is taken now, not in Phase 12.

One-directional evidence settles the rights number: seL4 has 4, Zircon 25,
CloudABI 31, Capsicum ~81. Capsicum's taxonomy outgrew a machine word and
broke its ABI: `cap_rights_t` is a multi-word struct spending 7 of every 64
bits on metadata, with an `init`/`set`/`is_set` API because two rights can no
longer be OR'd. WASI deleted its rights system in Preview 2 for creating "a
lot of ambiguity about how it mapped to POSIX semantics", which the "a right
is a distinction, not a syscall" rule prevents. managarm, adding fine-grained
rights ten years in, landed on ~14.

The refcounted table hedges a named precedent: Zircon's strict per-process
invariant was the first casualty of its own compatibility layer (RFC-0261
shares handle tables for Starnix). Free now; a redesign later.

No revocation: the one system that has it paid for it. seL4's recursive
revocation is preemptible and resumable, with progress stored in a zombie
capability, and a pre-mitigation worst-case execution time of about one
second, reduced to 436 µs computed and 81 µs observed. Object death plus a
strippable `TRANSFER` right covers the real cases. If revocation is ever
needed, a per-object **epoch bump is O(1)**, with no traversal, preemption
points or zombies, and the generation counter already is the epoch.

**Alternatives.** A per-process array with dense indices (rejected: it is the
POSIX promise, it serialises allocation, and Zircon does not have one); rights
as a multi-word struct with room to grow (rejected: that is Capsicum's
outcome, not its plan, and the growth broke the ABI); revocation now (rejected
on seL4's own numbers); the handle table inline in `Process` (rejected on
RFC-0261); quota at the frame layer (rejected by D-44: caller memory donation
is a permanent ABI decision, a quota revisable policy).

**Consequences.**

- `libs/handle-space` joins the `port_space`/`frame_space` pattern,
  host-tested with doctests. Per D-28 the two checks most likely to be
  silently unfailable are *the invalid value never decodes* and *a reused slot
  never yields the same value twice*; both get broken on purpose.
- The arena lives in `kernel`, not `wost`: it holds no raw pointer and touches
  no hardware, `005`'s own test.
- D-70 rules 7 and 9 are discharged for this interface.
- `002`'s open-questions list loses its first entry.
