# D-35 Enabling interrupts is safe because both preconditions are checked

Status: Accepted, 2026-08-14.

**Decision.** `interrupt::enable` is a safe function that asserts its
preconditions: the interrupt table is loaded, and the legacy controller is
silenced. It lives in `interrupt.rs`, not `cpu.rs`, beside the table that
makes it safe. A private `set_flag` serves both it and
`InterruptState::restore`.

**Rationale.** The kernel crate is `#![deny(unsafe_code)]`, and `005`
requires a `wost` abstraction to make the unsafe operation beneath it
*checkable*, not merely wrapped: an `unsafe fn` whose contract can only be
stated in prose is a guess. Both preconditions are readable values:
`table_is_loaded()` asks the CPU, `pic::is_silenced()` the driver. The
third, IRQ-safe locks, is a property of the code, not a runtime state, and
is D-34. Setting the flag the first time needs an argument; restoring an
already set flag does not.

**Consequences.**

- `cpu::disable_interrupts` moved to `interrupt::disable`, which returns the
  previous state so a critical section can restore instead of guess.
- `cpu.rs` and `interrupt.rs` reference each other. That is harmless within
  a crate; the alternative was `cpu` depending on the 8259 driver.
- Both asserts are reachable on a real machine: a kernel that cannot silence
  the 8259s or bring up the APIC declines to enable interrupts and says so,
  instead of reaching an assert.
