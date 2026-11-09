# D-120 A fault in the user copy resumes, because the program is the untrusted party

Status: Accepted (2026-08-24)

**Context.** D-97 built the user-copy ABI from the right premise — *the program
is the untrusted party* — and did not act on it. Every page of a range is
walked before a byte moves, so a bad pointer is refused rather than faulted on;
but the walk and the copy are two moments and **interrupts are on between
them**, so a thread preempted mid-copy whose pages are then unmapped faults on
memory that was there when it looked. That fault was terminal, which means a
program could stop the machine by arranging it. `011`'s close named it the last
correctness hole in the boundary.

**Decision.** An exception table, in the shape every kernel that has solved
this uses. The copy is hand-written assembly with the two instructions that can
fault between named labels, and a fixup label after them. A page fault whose
instruction pointer falls in that range resumes at the fixup, which returns how
many bytes were left; the caller turns that into `UserCopyError::Vanished`.

**Assembly rather than a loop**, because a compiler-generated
`copy_nonoverlapping` has no stable instruction range — nothing could say "a
fault *here* is recoverable" about one. Byte at a time, which is slower and is
the price: the ranges this kernel copies are a few bytes to a page, correctness
at the boundary is worth more than the difference, and a wider copy would need
one fixup per width.

**The redirect is `pub(super)` with exactly one caller.** "Set the instruction
pointer" is the primitive every exploit wants, and a general one is not
something this kernel should have. `usercopy::recover` is the only caller and
it has already established that the fault was inside its own two-instruction
range and that the target is its own fixup label. The narrowness is the whole
safety argument.

**The recovery is consulted *before* the ring-3 test.** This fault is taken in
ring 0 — the kernel is the one dereferencing — so `is_the_programs` says no and
the machine would stop. Getting the order wrong would leave the mechanism built
and unreachable, which is D-101's finding about the `VERW` in a new place.

**What it buys beyond the hole it closes.** Once a fault in the copy is
recoverable, dereferencing an arbitrary *user* address is memory-safe: the
worst outcome is a short copy and a number saying so. `copy_from_user_unwalked`
is that — range-checked so it can never touch the kernel's half, but not
walked. It is what Linux's `copy_from_user` has always been, and the walked
forms are kept because they give a **better answer**: a program that passed an
unmapped pointer learns that, rather than learning that some bytes did not move.

**The check had to be driven through the unwalked form**, and the reason is the
rule this project keeps re-learning. `admits` refuses an unmapped address
before a byte moves, so a check built on the walked copy tests the refusal it
already has and the recovery *not at all* — it would pass with the fixup
deleted. Pointing the unwalked form at nothing is the mechanism itself.

**Consequences.** 128 checks. `a_faulting_user_copy_resumes_and_reports` counts
recoveries rather than inferring one from the machine still running, requires
the kernel's half to be refused before the window opens, and requires a mapped
page to still copy — so a fixup that had simply broken the copy is not a pass.

Broken twice, and **both breaks panic rather than going red**: without the
fixup the fault is terminal and the self-test never reaches its verdict. The
harness treats a panic and a red verdict alike, which is right — a kernel that
died is a kernel that noticed — and `cargo xtask mutate` carries the mutation
with that noted, because a reader would otherwise expect a red check.

`unsafe` in `wost` is 354 lines against a budget of 380.

Left owed: nothing yet unmaps another thread's memory, so the window this
recovers from is not reachable on this machine — the count stays at one, from
the check that drives it deliberately. That is the honest state: the mechanism
is proved and the race it guards is not yet constructible. It becomes
constructible the moment a process has two threads.

**Amended by D-361.** On 2026-10-03 the budget stated above rose from 380 to
381 lines for the painted stack meter's two single-line volatile accesses.
`xtask/src/unsafety.rs` holds the current measurement.
