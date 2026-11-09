# D-90 The gate boots a machine the kernel cannot fully use

Status: Accepted (2026-08-24), extends D-65

**Context.** D-65 made the gate boot three kernels — the self-test image in
release and again in debug for overflow checks, and the shipped image — after an
audit found two configurations the gate never ran. All three boot on `-cpu max`,
where every feature the kernel asks for is present. So every precondition this
kernel checks has only ever been met.

`010` §10 named the specific case: x2APIC is assumed rather than diagnosed, and
`-cpu qemu64,-x2apic` produces a machine without it. D-33 chose x2APIC-only for
a good reason — xAPIC needs page tables that do not exist when the APIC is
brought up — which means there is no fallback to test, only a refusal.

**Decision.** A fourth boot: `cargo xtask degraded-check` runs the shipped image
on `-cpu qemu64,-x2apic` and requires the kernel to **name the missing feature
and still reach its steady state**. It joins `ci` after `boot-check`.

The bar is deliberately not "it works". It is that a machine the kernel cannot
fully use produces a diagnosis rather than a triple fault, and that the
diagnosis is on the serial log where a person will find it.

**Rationale.** A precondition that is only ever met is indistinguishable from
one that is never checked — which is D-28's own argument about checks, applied
to the configuration the checks run in. D-65 learned it once already: the gate
is only as wide as its configuration matrix.

This one is cheap because the kernel already degraded correctly; what was
missing was the evidence. Breaking it confirmed the gap was real: swallowing the
refusal in `start_interrupts` leaves a boot that looks exactly like a healthy
one, and nothing before this would have noticed.

`-cpu qemu64` also strips SMEP, SMAP and UMIP, so this boot exercises D-32's
reporting path at the same time — the reason `MACHINE` uses `-cpu max` at all
(D-65).

**Alternatives.** A self-test check for x2APIC (rejected: the self-test image
boots on `-cpu max`, so the check would assert a feature that is always there —
the unfailable-check hazard D-28 exists to prevent); asserting on the missing
feature and panicking (rejected: a kernel that will not boot on a machine it
could have diagnosed is worse than one that says why); testing every missing
feature (rejected for now: x2APIC is the one with no fallback, and a matrix of
them is a Phase 12 concern when real hardware is in the loop).

**Consequences.**

- The gate boots **four** kernels. D-65's title is now one short.
- `watch_serial` grew an optional second marker, so one reader serves both boot
  checks rather than two diverging copies.
- The break is to swallow the refusal, and the gate reports that the
  precondition is not being checked — naming the marker it wanted.
- Cost: one more QEMU boot in `ci`, about five seconds.
