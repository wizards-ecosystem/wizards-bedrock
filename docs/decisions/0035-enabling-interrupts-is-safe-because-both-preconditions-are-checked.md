# D-35 Enabling interrupts is safe because both preconditions are checked

Status: Accepted, 2026-08-14.

**Context.** `enable_interrupts` was written in Phase 1 and deleted unused,
on the grounds recorded in `005`: an `unsafe fn` whose contract can only be
stated in prose is a guess. But the kernel crate carries
`#![deny(unsafe_code)]`, so its replacement had to be safe to call.

**Decision.** `interrupt::enable` is a safe function that asserts its
preconditions: the interrupt table is loaded, and the legacy controller is
silenced. It lives in `interrupt.rs`, not `cpu.rs`, beside the table that
makes it safe. A private `set_flag` serves both it and
`InterruptState::restore`.

**Rationale.** `005` requires a `wost` abstraction to make the unsafe
operation beneath it *checkable*, not merely wrapped. The same rule is behind
`MmioRegion`'s bounds checks, `port::claim`'s registry and the private
terminal panic boundary; here it applies to an instruction. Both preconditions
are readable values: `table_is_loaded()` asks the CPU, `pic::is_silenced()`
the driver. The third, IRQ-safe locks, is a property of the code, not a
runtime state, and is D-34.

The split between `enable` and `set_flag` is the design in miniature: setting
the flag the first time needs an argument; restoring an already set flag
does not.

**Consequences.**

- `cpu::disable_interrupts` moved to `interrupt::disable`, which now returns
  the previous state so a critical section can restore instead of guess.
- `cpu.rs` and `interrupt.rs` reference each other. That is harmless within a
  crate; the alternative was `cpu` depending on the 8259 driver.
- Both asserts are reachable on a real machine: a kernel that cannot silence
  the 8259s or bring up the APIC declines to enable interrupts and says so,
  instead of reaching an assert.
