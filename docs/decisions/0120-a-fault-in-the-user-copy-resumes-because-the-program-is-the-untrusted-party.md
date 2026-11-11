# D-120 A fault in the user copy resumes, because the program is the untrusted party

Status: Accepted, 2026-08-24.

**Decision.** An exception table, the shape every kernel that solved this uses.
The copy is hand-written assembly with the two instructions that can fault
between named labels, and a fixup label after them. A page fault whose
instruction pointer falls in that range resumes at the fixup, which returns how
many bytes were left. The caller turns that into `UserCopyError::Vanished`.

**Assembly, not a loop**, because a compiler-generated `copy_nonoverlapping` has
no stable instruction range, so nothing could say "a fault *here* is
recoverable" about one. It copies a byte at a time, which is slower and is the
price: the ranges are a few bytes to a page, boundary correctness outweighs the
speed, and a wider copy would need one fixup per width.

**The redirect is `pub(super)` with exactly one caller.** "Set the instruction
pointer" is the primitive every exploit wants, and this kernel should have no
general one. `usercopy::recover`, the only caller, has already established that
the fault was inside its own two-instruction range and that the target is its
own fixup label. That narrowness is the safety argument.

**The recovery is consulted *before* the ring-3 test.** This fault is taken in
ring 0, because the kernel is the one dereferencing, so `is_the_programs` says
no and the machine would stop. The wrong order would leave the mechanism built
and unreachable.

**Once a fault in the copy is recoverable, dereferencing an arbitrary *user*
address is memory-safe**: the worst outcome is a short copy and a number saying
so. `copy_from_user_unwalked` is that, range-checked so it never touches the
kernel's half, but not walked. The walked forms stay because they give a
**better answer**: a program that passed an unmapped pointer learns that, not
just that some bytes did not move.

**Rationale.** D-97's walk and copy are two moments, and **interrupts are on
between them**: a thread preempted mid-copy whose pages are then unmapped
faults on memory that was there when it looked. A terminal fault there let a
program stop the machine by arranging it.

**Consequences.** `a_faulting_user_copy_resumes_and_reports` is driven through
the unwalked form, because `admits` refuses an unmapped address before a byte
moves and a check on the walked copy would pass with the fixup deleted. It
counts recoveries instead of inferring one from the machine still running, and
requires the kernel's half to be refused before the window opens and a mapped
page to still copy.

Breaking it panics rather than going red: without the fixup the fault is
terminal and the self-test never reaches its verdict. The harness treats a
panic like a red verdict, correctly: a kernel that panicked noticed the fault.
`cargo xtask mutate` carries the mutation with that noted, because a reader
would otherwise expect a red check.

`unsafe` in `wost` is 354 lines against a budget of 380.

Left owed: nothing yet unmaps another thread's memory, so the window this
recovers from is not reachable on this machine. The mechanism is proved and the
race it guards is not yet constructible. It becomes constructible the moment a
process has two threads.

**Amended by D-361.** On 2026-10-03 the budget stated above rose from 380 to
381 lines for the painted stack meter's two single-line volatile accesses.
`xtask/src/unsafety.rs` holds the current measurement.
