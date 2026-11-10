# D-29 Interrupt entry is generated assembly, not a nightly ABI

Status: Accepted, 2026-08-13.

**Context.** Phase 2 needs 256 interrupt gates. Rust cannot express a
handler directly: the calling convention that would allow it,
`extern "x86-interrupt"`, is gated behind nightly's `abi_x86_interrupt`, and
D-01 keeps this project on stable.

**Options.** (a) Adopt nightly for that one feature. (b) Take the `x86_64`
crate, which wraps the ABI and ships a full descriptor-table API. (c) Write
the entry path in assembly ourselves.

**Decision.** (c). One `global_asm!` block generates 256 stubs with `.rept`,
each padded to sixteen bytes, so the table is indexed by vector instead of
naming 256 functions. Each stub normalises the frame: it pushes a zero where
the CPU pushes no error code, then its own vector number. It then jumps to a
common tail that saves the fifteen general-purpose registers and passes a
frame pointer to a Rust dispatcher.

**Rationale.** Nightly for one feature costs the whole stable-channel
guarantee. The `x86_64` crate is the pragmatic choice in most projects, but
it inverts D-13: the `unsafe` defining this kernel's TCB would live in a
dependency, not in `wost`. The framekernel argument is that the privileged
surface is small, ours and reviewable. The entry path is also the
part of Phase 2 most worth understanding by hand.

**Consequences.**

- **The stub stride is an assumption, so it is checked.** Two symbols
  bracket the generated table, and `init` asserts the span is exactly
  `256 × 16` bytes before indexing into it.
- **`TrapFrame`'s field order and the assembly must agree**, and no compiler
  can check that. The selftest does, on the machine: it plants a breakpoint
  and checks that the handler saw the vector, error code, code selector,
  flags and stack pointer the CPU pushed. Swapping two fields turned the
  harness red.
- The dispatcher reads its handler from an `AtomicPtr` written before the
  table is loaded, so a trap that reaches it always has a handler to call.
- Not handled yet, each with its first caller still to come: `swapgs`
  (needs ring 3), `XSAVE` (needs a scheduler; the kernel is built
  `+soft-float` and cannot disturb SSE state), and nested-interrupt policy
  (needs interrupts on).
