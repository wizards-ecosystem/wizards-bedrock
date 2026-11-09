# D-35 Enabling interrupts is safe because both preconditions are checked

Status: Accepted (2026-08-14)

**Context.** `enable_interrupts` was written in Phase 1 and deleted unused, on
the grounds recorded in `005`: an `unsafe fn` whose contract can only be stated
in prose is a guess. But the kernel crate carries `#![deny(unsafe_code)]`, so
whatever replaced it had to be callable from safe code.

**Decision.** `interrupt::enable` is a safe function that asserts its
preconditions: the interrupt table is loaded, and the legacy controller has been
silenced. It lives in `interrupt.rs`, not `cpu.rs`, next to the table that is
the reason it is safe. A private `set_flag` serves both it and
`InterruptState::restore`.

**Rationale.** `005` requires that a `wost` abstraction make the unsafe
operation underneath it *checkable*, not merely wrapped — the rule behind
`MmioRegion`'s bounds checks, `port::claim`'s registry, and the private terminal
panic boundary. This is the same rule applied to an instruction. Both
preconditions are already values something can read: `table_is_loaded()` asks
the CPU, and `pic::is_silenced()` asks the driver. The third — that locks are
IRQ-safe — is a property of the code rather than a runtime state, and is D-34.

The split between `enable` and `set_flag` is the design in miniature: setting
the flag the first time needs an argument, and putting back a flag that was
already set does not.

**Consequences.**

- `cpu::disable_interrupts` moved to `interrupt::disable`, which now returns the
  previous state so a critical section can restore rather than guess.
- `cpu.rs` and `interrupt.rs` reference each other. Harmless within a crate, and
  the alternative was `cpu` depending on the 8259 driver.
- Both asserts are reachable on a real machine — a kernel that cannot silence
  the 8259s or bring up the APIC declines to enable interrupts and says so,
  rather than reaching an assert.
