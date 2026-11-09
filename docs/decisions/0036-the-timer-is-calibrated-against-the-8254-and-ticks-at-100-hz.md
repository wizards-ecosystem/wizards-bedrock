# D-36 The timer is calibrated against the 8254, and ticks at 100 Hz

Status: Accepted (2026-08-20)

**Context.** The APIC timer counts the bus clock divided by a configurable
factor, and nothing reports that clock's rate. Phase 2 owns no page tables, so
every memory-mapped clock is behind the wall D-33 hit. The slice's
deliverables — a periodic tick and an uptime — both need the rate.

**Options.**

- *CPUID leaf 15H, or the hypervisor leaves.* Absent or zero under QEMU's
  software emulation; not trustworthy under VMware. A calibration source that
  works on neither test target is not one.
- *The HPET.* Memory-mapped at a device hole the bootloader does not map —
  D-33's constraint verbatim.
- *TSC-deadline mode.* Relocates the same problem: the TSC's rate is reported
  by the same leaves that are absent, so it would need the same 8254
  measurement first, plus invariant-TSC reasoning on top.
- *The 8254 over port I/O.* Present in every PC and every hypervisor, its
  crystal fixed at 1.193182 MHz since 1981, and ports are already safe to
  drive from the kernel.
- *Go tickless now.* D-15's destination, but a tickless kernel decides when
  next to wake, and that decision belongs to a scheduler that does not exist.

**Decision.** The 8254: counter 0, mode 0, one twenty-millisecond window,
bracketed symmetrically, divide-by-one, rounded to nearest once from the raw
pair. The result must land between 1 MHz and 50 GHz, with three attempts
before the kernel boots without a clock — and a timer failure vetoes nothing
else. The tick is periodic at 100 Hz; uptime interpolates the countdown
between ticks, sandwich-read and clamped monotonic. Calibration runs between
`apic::init` and `interrupt::enable`, unpreemptible by ordering alone. The
kernel's idle becomes halt–work–halt.

**Rationale.** The window sits between two hard bounds: the sixteen-bit
counter wraps ambiguously at 54.9 ms, and per-access jitter grows as the
window shrinks; 20 ms costs parts in ten thousand and keeps a 35 ms stall
margin. Channel 0 over the channel-2 speaker trick because its gate is always
high and its one output edge lands on a controller slice 2 masked — no port
`0x61`, no second device. The divide factor never enters the correctness
path: the count is derived from the same divided clock the window measured,
so the tick rate survives even a misfired divide write.

**Consequences.**

- The 8254 is programmed once and **parked**: a control word with no count
  halts it, and the permanent port claim means nothing can ever start it
  again. IRQ 0 is never unmasked, which also sidesteps every hypervisor's
  tick-coalescing machinery.
- Accuracy is one window's worth — roughly a part in a thousand — which
  affects only long-run wall-clock drift that nothing yet asserts.
- A critical section longer than a tick can swallow one (the APIC holds a
  single pending interrupt), and uptime undercounts by 10 ms. Accepted under
  D-34's recorded latency cost; Phase 4's tickless design is the fix.
- 100 Hz and periodic mode are explicitly disposable when D-15 lands; the
  vision doc counts wakeups as a feature, and this spends a hundred a second
  until a scheduler can spend fewer.
- `wost` gained five timer functions and zero new `unsafe` blocks: everything
  rides the MSR helpers whose gate D-33 built.

**Amended 2026-08-20**, after the phase's adversarial audit, in which two
independent reviewers converged on the same hole: the believability bounds
cannot catch a stall that crosses the counter's 54.9 ms span mid-window,
because the fold inflates the rate by a factor of two to four — inside
bounds that must span both targets' clocks. The window is now guarded in
depth: no gap between counter observations may exceed ten milliseconds, the
finished window must land within one such gap of its target, and two
consecutive windows must agree to one part in sixty-four — no single guard
survives a stall of exactly the wrong length, but no stall forges the same
wrong answer twice. A fourth guard came from the datasheet: the 8254 loads
a written count only on the crystal edge after its high byte, so the
baseline read now waits on the read-back status byte's null-count flag —
instant under both hypervisors, real on real hardware. Attempts rose from
three to five, giving the agreeing pair room around discarded windows.

**Superseded in part by D-57 (2026-08-23).** The tick, the interpolated
uptime and the monotonic clamp are gone; what survives is the measurement —
the same window, the same guards, the same agreeing pair — now applied to two
clocks instead of one. The two debts this recorded are discharged there.
