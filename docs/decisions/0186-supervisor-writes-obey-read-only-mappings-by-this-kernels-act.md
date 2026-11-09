# D-186 Supervisor writes obey read-only mappings by this kernel's act

Status: Accepted, 2026-08-27. Amends D-12 and closes PW-300.

**Context.** Kernel text and read-only data have read-only leaves, but x86
allows supervisor writes through those leaves while `CR0.WP` is clear. The
kernel neither set nor read that bit, so the W^X claim depended on firmware or
the bootloader leaving it set. A readback added in isolation would preserve the
same dependence because contemporary firmware commonly does leave it set.

**Decision.** Write protection is an unconditional member of the CPU
protection contract: unlike the optional CPUID-advertised CR4 features,
`CR0.WP` exists on every supported x86-64 processor. `enable_protections` sets
it before the kernel builds and activates the W^X tables, and `Protections`
reads it back alongside NX, SMEP, SMAP and UMIP.

The machine witness is driven rather than observational. With interrupts
excluded, it clears WP, confirms the clear, and reaches the same private setter
boot uses. It then attempts one assembly write which stores a byte's existing
value back to a private `.rodata` symbol. The trap path recovers only when the
saved privilege is ring 0, the page-fault error is exactly present-page write,
and both the instruction and `CR2` match the probe's private labels. Any other
kernel fault remains terminal. If the driven setter fails, the diagnostic path
repairs WP independently before returning a red result.

**Rationale.** Setting the bit makes page-table write permission meaningful to
the supervisor; driving it through an off/on transition proves that this
kernel, rather than inherited state, owns the result. Writing back the byte
already present means the deliberate mutation which omits WP can return a red
verdict without corrupting the image. The exact recovery window does not turn
arbitrary kernel protection faults into resumable events.

**Alternatives.** Trusting the bootloader was the defect. Readback alone could
not distinguish inherited state from the kernel's action. Making all read-only
data executable or writable to avoid the probe would surrender the invariant
being checked. A general kernel fault-fixup API was rejected because redirecting
an arbitrary live trap frame is authority only the narrow assembly window can
justify.

**Consequences.** The boot log now includes `wp`. The machine check
`supervisor writes obey read-only pages` proves the driven write and the actual
fault consequence. Its persistent mutation removes the bit from the common
setter; the driven transition makes that mutation deterministic even when
firmware arrived with WP already enabled.
