# D-97 The user copy is the boundary, and its window is three instructions wide

Status: Accepted (2026-08-24)

**Context.** D-81 named `stac`/`clac` as *"a scoped RAII guard; it **is** the
user-copy ABI"*, and listed SMAP among the features to shape before ring 3. Ring
3 exists now (D-96), so there is user memory to copy and a reason to copy it.

**Decision.** Two functions, `copy_from_user` and `copy_to_user`, and nothing
else in this kernel may open the SMAP window. The guard is a private RAII type
whose whole life is one `copy_nonoverlapping`: an open window is the CPU
agreeing to dereference whatever a user pointer happens to be, and the shorter
it is, the smaller the class of bugs that can be an escalation rather than a
fault.

**Range-checked, never `>> 47 == 0`** (D-83). Every page of the range is walked
before a byte moves — present, `USER`, and writable where the copy writes — and
a range that starts inside user space and ends outside it is refused, which the
shift cannot see. A user pointer naming kernel memory, or nothing, is a refusal
rather than a fault: a fault here is terminal (D-31), and "the program passed a
bad pointer" is not a reason to stop the machine.

Where SMAP is absent the window does nothing, because `stac` is a `#UD` on a
processor without it — which the degraded boot (D-90) exercises.

**The release build caught a bug the debug build could not.** The first version
gave `stac` and `clac` the options `nomem` and `preserves_flags`. Both are
wrong, and wrong in a way that only optimisation reveals:

- **`nomem` is a lie about the instruction's purpose.** `stac` changes whether a
  *later memory access faults*. Telling the compiler it touches no memory lets
  it move the copy out of the window — which it did, in release only, and the
  machine took a protection violation reading the page it had just been given
  permission for.
- **`preserves_flags` is a lie about its name.** The flag it sets is
  `EFLAGS.AC`.

The debug build passed. D-65 made the gate boot both profiles after an audit
found configurations it never ran, and the reason recorded then was overflow
checks; this is the same lesson arriving through a different door.

**Rationale.** The refusals are the half worth checking, and the check drives
all five: kernel memory named as the user's, a range straddling the end of user
space, a range that wraps, an unmapped page, and a write to the program's own
code page — which is user-readable, executable and read-only, because W^X holds
for user memory too.

Two of those breaks behave differently and both are informative. Replacing the
range check with `>> 47 == 0` turns the check red on the straddling case, which
is precisely the failure D-83 predicted when it forbade the idiom. Removing the
writability check produces a page fault instead — the hardware confirming the
mapping really is read-only, which is better evidence than an assertion and a
worse break, because a crash says less than a failure (D-28).

**Alternatives.** A public window type callers open around their own accesses
(rejected: it is the ABI, and an ABI with two users is an ABI with two
disciplines); trusting the caller's bounds and letting a bad pointer fault
(rejected: terminal, and the program is the untrusted party); a shift instead of
a range check (rejected by D-83, and the check that catches it is now in the
tree).

**Consequences.**

- Ninety-eight named machine checks.
- The copy reads the very instructions the CPU executed in ring 3, through the
  *user* mapping — stronger than a round trip through memory the kernel also
  wrote.
- `USER_END` is where user memory stops, named once. Nothing computes it.
- Still owed: a fault *during* a copy is terminal, where a real kernel would
  recover and return an error. That needs a fixup table keyed on the faulting
  instruction pointer, and it needs the trap dispatch D-81 built to be able to
  consult one. It is not needed until a program can pass a pointer that races
  its own unmapping, which is slice 7's problem at the earliest.
