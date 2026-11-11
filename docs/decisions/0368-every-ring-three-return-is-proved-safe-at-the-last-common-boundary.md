# D-368 Every ring-three return is proved safe at the last common boundary

Status: Accepted, 2026-10-09. Amends D-102 and D-348.

**Decision.** One pure `x86-tables` value validates a prospective user return:
the code and stack selectors must be the exact installed ring-three pair;
`RIP` and `RSP` must lie in the admitted low user half; and `RFLAGS` may carry
only architecturally supported user bits, with its always-one and interrupt
bits present and `NT`, `VM`, `IOPL`, virtual-interrupt and reserved bits clear.
`wost` additionally requires the saved `RIP` to remain user-executable in the
live address space.

The check runs after the registered handler returns and immediately before
the common assembly tail consumes the frame. This position covers calls,
fault recovery, hardware interrupts, preemption and a trap frame resumed after
a context switch; device interrupts have already been dismissed, and no
interruptible Rust path remains between validation and `iretq`. A refused
return invokes a separately registered diverging handler with the still-live
trap identity. The kernel records the exact refusal and retires only that
program through the ordinary trap-side scheduler path. It never attempts the
malformed `iretq` and never reclassifies it as a kernel fault.

The CPU-pushed 22-word frame remains unchanged. Validation does not sanitize
attacker state into a different program state, and it does not require the
user stack to be mapped: `iretq` does not dereference `RSP`, so a canonical
unmapped stack remains legitimate and any later access faults in ring three.
Exact selectors and immutable descriptor tables, an executable canonical
`RIP`, a canonical low-half `RSP`, and the flags allowlist cover every operand
the 64-bit `iretq` consumes. A broad `#GP` recovery arm is therefore not added;
it would turn unrelated kernel faults into process faults and weaken D-14.

**Rationale.** Deep Codex Security scan
`32245094-b8b7-47d1-b6e7-a52554800772` of revision
`c746707bdadb714ac5ac4071e7f61e167b66aa3b` found that a program can load a
non-canonical `RSP`, or set `RFLAGS.NT`, and enter the public `INT 0x80` gate;
the returning call's common `iretq` then raises `#SS` or `#GP` attributed to
ring zero and halts the whole kernel. D-348 prevents a mapped instruction from
ending in the canonical hole, but it does not constrain a live register after
first entry.

The scan's claim that a same-privilege interrupt supplies only three words and
that copying `SS:RSP` overruns its frame is false for this 64-bit-mode path:
Intel SDM Vol. 3A section 6.14.2 says 64-bit mode pushes `SS:RSP`
unconditionally, and the live breakpoint witness proves the 22-word frame ends
at the exact pre-interrupt `RSP` with the installed kernel `SS`. Shortening
that frame would break the architecture rather than harden it. The scan's
failed provider-side publication of finding 23 neither makes the valid return
path safe nor authorizes rewriting provider state; its canonical bundle
remains partial and every other reported issue is separately triaged before a
source change.

**Alternatives.** Checking only the syscall branch misses timer and resumed
trap returns. Clearing `NT` or rewriting a bad stack silently changes attacker
state and leaves other invalid fields reachable. Letting the eventual fault
enter the generic handler loses the originating CPL and recreates the panic.
Replacing the full frame with a legacy-mode short frame contradicts the Intel
64 contract and the live probe.

**Consequences.** The live same-privilege frame witness must remain green.
Mutations 1,645 through 1,648 break each pure return-state test at its field
guard. Mutations 1,649 and 1,650 replace the non-canonical stack and `NT`
machine behaviours with the ordinary program; each fails only its named
containment check, which proves the exact process reaches last rites while
another program, timer interrupts and the kernel continue.
