# D-29 Interrupt entry is generated assembly, not a nightly ABI

Status: Accepted, 2026-08-13.

**Decision.** Write the interrupt entry path in assembly, rather than adopt
nightly's `abi_x86_interrupt` or take the `x86_64` crate. One `global_asm!`
block generates 256 stubs with `.rept`, each padded to sixteen bytes, so the
table is indexed by vector instead of naming 256 functions. Each stub
normalises the frame: it pushes a zero where the CPU pushes no error code,
then its own vector number. It then jumps to a common tail that saves the
fifteen general-purpose registers and passes a frame pointer to a Rust
dispatcher.

**Rationale.** Nightly for one feature costs the whole stable-channel
guarantee (D-01). The `x86_64` crate inverts D-13: the `unsafe` defining
this kernel's TCB would live in a dependency, not in `wost`, and the
framekernel argument is that the privileged surface is small, ours and
reviewable.

**Consequences.**

- **The stub stride is an assumption, so it is checked.** Two symbols
  bracket the generated table, and `init` asserts the span is exactly
  `256 × 16` bytes before indexing into it.
- **`TrapFrame`'s field order and the assembly must agree**, and no compiler
  can check that. The selftest does, on the machine: it plants a breakpoint
  and checks that the handler saw the vector, error code, code selector,
  flags and stack pointer the CPU pushed.
- The dispatcher reads its handler from an `AtomicPtr` written before the
  table is loaded, so a trap that reaches it always has a handler to call.
- Not handled at adoption: `swapgs` (needs ring 3), XSAVE (needs a
  scheduler; the kernel is built `+soft-float` and cannot disturb SSE state)
  and nested-interrupt policy (needs interrupts on).
