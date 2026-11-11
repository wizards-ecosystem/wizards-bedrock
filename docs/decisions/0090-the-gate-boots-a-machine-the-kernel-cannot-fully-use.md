# D-90 The gate boots a machine the kernel cannot fully use

Status: Accepted, 2026-08-24. Extends D-65.

**Decision.** A fourth boot: `cargo xtask degraded-check` runs the shipped image
on `-cpu qemu64,-x2apic` and requires the kernel to **name the missing feature
and still reach its steady state**. It joins `ci` after `boot-check`.

The bar is not "it works". A machine the kernel cannot fully use must produce a
diagnosis, not a triple fault, on the serial log where a person will find it.
D-33 chose x2APIC-only because xAPIC needs page tables that do not exist when
the APIC comes up, so there is no fallback to test, only a refusal.

**Rationale.** A precondition that is only ever met cannot be told apart from
one that is never checked. That is D-28's argument, applied to the configuration
the checks run in: every boot ran on `-cpu max`, so every precondition had only
ever been met. Swallowing the refusal in `start_interrupts` leaves a boot that
looks healthy, which nothing before this would have noticed. `-cpu qemu64` also
strips SMEP, SMAP and UMIP, so this boot exercises D-32's reporting path too.

**Alternatives.** A self-test check for x2APIC (rejected: the self-test image
boots on `-cpu max`, so the check could never fail, the hazard D-28 exists to
prevent). Panicking on the missing feature (rejected: refusing a machine the
kernel could have diagnosed is worse than saying why). Testing every missing
feature (rejected for now: x2APIC is the one with no fallback, and a matrix of
them is a Phase 12 concern when hardware is in the loop).

**Consequences.**

- The gate boots **four** kernels. D-65's title is now one short.
- The break swallows the refusal, and the gate reports that the precondition is
  not being checked, naming the line it wanted.
