# D-163 The handle table works through a shared reference, and a promise is a value

Status: Accepted, 2026-08-25.

**Context.** Phase 6 slice 1 owes D-151: a completion must be able to carry a
handle. Two things stood in the way, and the second was not obvious until the
first was fixed.

**A program's table could not be written to after it was spawned.** `spawn`
built a `HandleTable`, inserted the bootstrap channel and wrapped it in an
`Arc`, and every mutating method took `&mut self` — so from that moment the
table was frozen. The ring is served from the syscall handler, where no
`&mut` to it exists and none can, because the table is reached through a
`BOOTSTRAPS` guard that must be dropped before anything else is taken (D-121).

**Wrapping it in a lock is the obvious answer and the wrong one.** `insert`
takes the handle arena's leaf guard. A table lock taken as a leaf and held
across that is leaf-under-leaf, which is what `lock_leaf` forbids and what the
machine check watches for; a token-bearing lock cannot be taken in trap context
at all, because `ThreadContext::claim` returns `None` inside a handler (D-55).
There is no lock that fits.

**Decision.** The count becomes an atomic and every method takes `&self`.

`id`, `mixer` and `quota` were already immutable after construction. The only
mutable state was `live: usize`, and it becomes one `AtomicU64` holding the two
counts `libs/handle-space`'s `Quota` packs — handles held, and slots promised.
No lock is added, no lock order changes, and `Arc<HandleTable>` becomes usable
exactly where it already was.

**A reservation is a value with a destructor, not a rule.** `reserve_slot`
returns a `SlotReservation` holding an `Arc` to the table. `install` consumes
it and keeps it; every other end — a refusal, a cancellation, a `?` on the way
through, the death of the program holding it — runs `Drop`, which gives the
promise back.

**Rationale.** D-151 asks that "the reservation is returned on every terminal
path", and names three paths. The trouble with a property stated as a list of
paths is that the list is a copy of the control flow, and it stops being a copy
the moment a path is added — which is D-157's rule one level down. Making it a
destructor obligation means the paths do not have to be enumerated in order to
be covered: there is no way to lose one, including down a path nobody has
written yet. Slice 3's dead-program path is the third in D-151's list and needs
no new code here to be correct.

The atomic is also what makes the two counts one word. Moving a promise to a
handle is a single compare-and-exchange, so no reader ever sees a table that
holds neither — which is what a quota check would otherwise be racing.

**Consequences.**

- `libs/handle-space` gains `Quota`: `reserve`, `commit`, `release`, `take`,
  `close`, all total, all host-tested, five deliberate breaks. The overflow
  test found a real edge on the first run — `reserve` could carry a full
  promise half into the held half — and the transitions are now guarded on the
  half being incremented as well as on the limit.
- `insert` splits: `fill` does the arena work and touches no count, so the two
  callers that charge differently share the part that does not.
- **One existing mutation had to be disambiguated.** `install`'s attenuation is
  the same line as `accept`'s, so `xtask/src/mutate.rs`'s one-site rule started
  failing — correctly, and immediately. The pattern now carries its next line.
- `live()` counts held handles and not promises. A check asking what a program
  can *reach* must not count what it has been promised, and the two are
  separately readable for exactly that reason.
