# D-163 The handle table works through a shared reference, and a promise is a value

Status: Accepted, 2026-08-25.

**Decision.** For D-151, a program's `HandleTable` must be writable after
spawn from the syscall handler, where no `&mut` exists because the table is
reached through a `BOOTSTRAPS` guard that must be dropped first (D-121). The
count becomes an atomic and every method takes `&self`. `id`, `mixer` and
`quota` were already immutable; the only mutable state, `live: usize`, becomes
one `AtomicU64` holding the two counts `libs/handle-space`'s `Quota` packs,
handles held and slots promised. No lock is added, no lock order changes, and
`Arc<HandleTable>` is usable exactly where it already was.

A lock does not fit. `insert` takes the handle arena's leaf guard, so a table
lock taken as a leaf is leaf-under-leaf, which `lock_leaf` forbids; a
token-bearing lock cannot be taken in trap context, because
`ThreadContext::claim` returns `None` inside a handler (D-55).

**A reservation is a value with a destructor, not a rule.** `reserve_slot`
returns a `SlotReservation` holding an `Arc` to the table. `install` consumes
it and keeps it; every other end (a refusal, a cancellation, a `?` on the way
through, the death of the program holding it) runs `Drop`, which gives the
promise back.

**Rationale.** D-151 asks that "the reservation is returned on every terminal
path" and names three. A property stated as a list of paths is a copy of the
control flow and stops being one the moment a path is added, which is D-157's
rule one level down. A destructor obligation covers every path, including ones
nobody has written yet: slice 3's dead-program path, the third in D-151's list,
needs no new code here to be correct. The atomic makes the two counts one word,
so moving a promise to a handle is a single compare-and-exchange and no reader
sees a table that holds neither.

**Consequences.**

- `libs/handle-space` gains `Quota`: `reserve`, `commit`, `release`, `take`,
  `close`, all total and host-tested. Its transitions are guarded on the half
  being incremented as well as on the limit, because `reserve` could carry a
  full promise half into the held half.
- `insert` splits: `fill` does the arena work and touches no count, so the two
  callers that charge differently share the part that does not.
- `install`'s attenuation is the same line as `accept`'s, so the mutation
  pattern carries its next line to stay one-site.
- `live()` counts held handles and not promises. A check asking what a program
  can *reach* must not count what it has been promised, and the two are
  separately readable for that reason.
