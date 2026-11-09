# D-78 The handle table is one arena, and a handle is arithmetic

Status: Accepted, 2026-08-24.

**Context.** `002-architecture.md` listed "handle table representation and
rights taxonomy" as owed before Phase 5. `010` §3 answered the taxonomy half —
16–24 rights in one `u32`, and a right must name a distinction the kernel makes
about the object rather than a syscall. What remained was the layout and the
lifetime rules, and they must be settled before the first object exists,
because the first caller freezes them.

**Decision.** One global arena, not a per-process array. A slot's base value is
`generation ‖ arena_index`; the value a process sees is
`(base_value << 2 | 0b11) ^ per_table_mixer`. Lookup is arithmetic plus one
ownership comparison. The encode/decode arithmetic goes to `libs/handle-space`;
the kernel keeps the arena and the lock.

- **No lowest-free promise.** The reserved low bits forbid a dense encoding, so
  the promise is unmakeable rather than merely unmade.
- **`HandleTable { table: AtomicPtr<HandleArray> }`** — Linux's
  `files_struct`/`fdtable` split, so readers can go lock-free later without
  touching a call site.
- **The table is a refcounted object**, not an inline field of `Process`.
- **Rights are 16–24 bits of one `u32`**, universal (`DUPLICATE`, `TRANSFER`,
  `WAIT`, `INSPECT`) split from per-type.
- **Type encodings have headroom for 64 object types.** Policy is monotonic and
  frozen — tighten-only, immutable once a process has children — with bits
  encoded so adding a type never renumbers an existing one.
- **Refcounts use `inc_not_zero` from day one.** Close does not promise global
  visibility on return.
- **No revocation.**
- The per-process **quota** D-44 assigned to this doc is answered at this
  layer: a process's kernel memory is bounded by what its handle table can
  name.

**Rationale.** The encoding is Zircon's, and two independent research angles
converged on copying it without sight of each other — one for security, because
generation tagging kills use-after-free on handle reuse; one for scalability,
via the Scalable Commutativity Rule. Zircon's contract commutes *by
construction*. POSIX's "lowest free descriptor" does not: it is an ordering
promise that serialises allocation, and sv6 measured the unordered alternative
scaling linearly with **no single-core penalty** — "at one core, both cases
perform identically." That is the rare case where the scalable choice is free
today, which is why it is taken now rather than in Phase 12.

The rights number is settled by one-directional evidence: seL4 has 4, Zircon
25, CloudABI 31, Capsicum ~81. Capsicum's taxonomy outgrew a machine word and
broke its ABI — `cap_rights_t` is a multi-word struct spending 7 of every 64
bits on metadata, with an `init`/`set`/`is_set` API because two rights can no
longer be OR'd. WASI deleted its rights system entirely in Preview 2 for
creating "a lot of ambiguity about how it mapped to POSIX semantics", which is
exactly what the "a right is a distinction, not a syscall" rule prevents.
managarm, adding fine-grained rights ten years in, landed on ~14.

The refcounted table is a hedge with a named precedent: Zircon's strict
per-process invariant was the first casualty of its own compatibility layer
(RFC-0261 shares handle tables for Starnix). Free now; a redesign later.

No revocation, because the one system that has it paid for it. seL4's recursive
revocation is preemptible and resumable with progress stored in a zombie
capability, and a pre-mitigation worst-case execution time of about one second,
reduced to 436 µs computed and 81 µs observed. Object death plus a strippable
`TRANSFER` right covers the real cases, and if revocation is ever needed, a
per-object **epoch bump is O(1)** — no traversal, no preemption points, no
zombies — and the epoch already exists as the generation counter.

**Alternatives.** A per-process array with dense indices (rejected: it is the
POSIX promise, it serialises allocation, and Zircon does not have one); rights
as a multi-word struct with room to grow (rejected: that is Capsicum's outcome,
not its plan, and the growth is what broke the ABI); revocation now (rejected on
seL4's own numbers); the handle table inline in `Process` (rejected on
RFC-0261); quota at the frame layer (rejected by D-44 — making callers donate
memory is an ABI decision and permanent, while a quota is policy and stays
revisable).

**Consequences.**

- `libs/handle-space` joins the `port_space`/`frame_space` pattern, host-tested
  with doctests. Per D-28 the two checks most likely to be silently unfailable
  are *the invalid value never decodes* and *a reused slot never yields the
  same value twice*; both get broken deliberately.
- The arena lives in `kernel`, not `wost`: it holds no raw pointer and touches
  no hardware, which is `005`'s own test.
- D-70 rules 7 and 9 are discharged for this interface.
- `002`'s open-questions list loses its first entry.
