# D-97 The user copy is the boundary, and its window is three instructions wide

Status: Accepted, 2026-08-24.

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

**`stac` and `clac` do not take `nomem` or `preserves_flags`.** `stac` changes
whether a *later memory access faults*, so `nomem` lets the compiler move the
copy out of the window (it did, in release only), and the flag it sets is
`EFLAGS.AC`. The debug build passed; this is D-65's lesson that the gate boots
both profiles.

**Rationale.** The refusals are the half worth checking, and the check drives
all five: kernel memory named as the user's, a range straddling the end of user
space, a wrapped range, an unmapped page, and a write to the program's own
read-only code page, because W^X holds for user memory too. Replacing the range
check with `>> 47 == 0` turns it red on the straddling case, the failure D-83
predicted when it forbade the idiom.

**Alternatives.** A public window type callers open around their own accesses
(rejected: it is the ABI, and an ABI with two users has two disciplines).
Trusting the caller's bounds and letting a bad pointer fault (rejected:
terminal, and the program is the untrusted party). A shift instead of a range
check (rejected by D-83, and the check that catches it is now in the tree).

**Consequences.**

- `USER_END` is where user memory stops, named once. Nothing computes it.
- Still owed: a fault *during* a copy is terminal instead of returning an error.
  That needs a fixup table keyed on the faulting instruction pointer, and the
  trap dispatch D-81 built must be able to consult one. It is not needed until a
  program can pass a pointer that races its own unmapping, slice 7's problem at
  the earliest.
