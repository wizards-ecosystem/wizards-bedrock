# D-97 The user copy is the boundary, and its window is three instructions wide

Status: Accepted, 2026-08-24.

**Context.** D-81 named `stac`/`clac` as *"a scoped RAII guard; it **is** the
user-copy ABI"*, and listed SMAP among the features to shape before ring 3. Ring
3 exists now (D-96), so there is user memory to copy and a reason to copy it.

**Decision.** Two functions, `copy_from_user` and `copy_to_user`, and nothing
else in this kernel may open the SMAP window. The guard is a private RAII type
whose whole life is one `copy_nonoverlapping`. An open window lets the CPU
dereference any user pointer, so the shorter it is, the fewer bugs escalate
instead of faulting.

**Range-checked, never `>> 47 == 0`** (D-83). Every page of the range is walked
before a byte moves: present, `USER`, and writable where the copy writes. A
range that starts inside user space and ends outside it is refused, which the
shift cannot see. A user pointer naming kernel memory, or nothing, is refused,
because a fault here is terminal (D-31) and a bad pointer is no reason to stop
the machine.

Where SMAP is absent the window does nothing, because `stac` is a `#UD` on a
processor without it, as the degraded boot (D-90) exercises.

**The release build caught a bug the debug build could not.** The first version
gave `stac` and `clac` the options `nomem` and `preserves_flags`, both wrong in
ways only optimisation reveals.

- **`nomem` misstates the instruction's purpose.** `stac` changes whether a
  *later memory access faults*. Telling the compiler it touches no memory lets
  it move the copy out of the window. It did, in release only, and the machine
  took a protection violation reading the page it had just been permitted.
- **`preserves_flags` contradicts its name.** The flag it sets is `EFLAGS.AC`.

The debug build passed. D-65 made the gate boot both profiles after an audit
found configurations it never ran, citing overflow checks; this is the same
lesson.

**Rationale.** The refusals are the half worth checking, and the check drives
all five. It names kernel memory as the user's, straddles the end of user space,
wraps a range, touches an unmapped page, and writes to the program's own code
page. That page is user-readable, executable and read-only, because W^X holds
for user memory too.

Two of those breaks behave differently. Replacing the range check with
`>> 47 == 0` turns the check red on the straddling case, the failure D-83
predicted when it forbade the idiom. Removing the writability check produces a
page fault instead: better evidence that the mapping is read-only, but a worse
break, because a crash says less than a failure (D-28).

**Alternatives.** A public window type callers open around their own accesses
(rejected: it is the ABI, and an ABI with two users has two disciplines).
Trusting the caller's bounds and letting a bad pointer fault (rejected:
terminal, and the program is the untrusted party). A shift instead of a range
check (rejected by D-83, and the check that catches it is now in the tree).

**Consequences.**

- Ninety-eight named machine checks.
- The copy reads the instructions the CPU executed in ring 3 through the *user*
  mapping, stronger than a round trip through memory the kernel also wrote.
- `USER_END` is where user memory stops, named once. Nothing computes it.
- Still owed: a fault *during* a copy is terminal instead of returning an error.
  That needs a fixup table keyed on the faulting instruction pointer, and the
  trap dispatch D-81 built must be able to consult one. It is not needed until a
  program can pass a pointer that races its own unmapping, slice 7's problem at
  the earliest.
