# D-78 The handle table is one arena, and a handle is arithmetic

Status: Accepted, 2026-08-24.

**Decision.** One global arena, not per-process arrays. A slot's base value is
`generation ‖ arena_index`; a process sees `(base_value << 2 | 0b11) ^
per_table_mixer`. Lookup is arithmetic and one ownership comparison. The
encode/decode arithmetic goes to `libs/handle-space`; the kernel keeps the
arena and the lock. This answers the layout and lifetime rules
`002-architecture.md` owed, before the first caller freezes them; `010` §3
answered the taxonomy.

- **No lowest-free promise.** The reserved low bits forbid a dense encoding, so
  the promise is unmakeable rather than merely unmade.
- **`HandleTable { table: AtomicPtr<HandleArray> }`**: Linux's
  `files_struct`/`fdtable` split, so readers can go lock-free later without
  touching a call site.
- **The table is a refcounted object**, not an inline field of `Process`.
- **Rights are 16 to 24 bits of one `u32`**, universal (`DUPLICATE`, `TRANSFER`,
  `WAIT`, `INSPECT`) split from per-type. A right must correspond to a
  distinction the kernel actually makes about that object, not a syscall.
- **Type encodings have headroom for 64 object types.** Policy is monotonic and
  frozen (tighten-only, immutable once a process has children); adding a type
  never renumbers an existing one.
- **Refcounts use `inc_not_zero` from day one.** Close promises no global
  visibility on return.
- **No revocation.**
- The per-process **quota** D-44 assigned to this doc is answered here: a
  process's kernel memory is bounded by what its handle table can name.

**Rationale.** The encoding is Zircon's: generation tagging kills
use-after-free on handle reuse, and its contract commutes by construction,
where POSIX's "lowest free descriptor" is an ordering promise that serialises
allocation. sv6 measured the unordered alternative with no single-core
penalty, so the scalable choice is free today and taken now, not in Phase 12.
Capsicum's rights outgrew a machine word and broke its ABI, and WASI deleted
its rights for ambiguity against POSIX; the distinction rule prevents both.
The refcounted table hedges Zircon's per-process invariant, the first casualty
of its own compatibility layer (RFC-0261). Revocation is omitted on seL4's
cost; object death plus a strippable `TRANSFER` right covers the real cases,
and if it is ever needed a per-object epoch bump is O(1), and the generation
counter already is the epoch.

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
