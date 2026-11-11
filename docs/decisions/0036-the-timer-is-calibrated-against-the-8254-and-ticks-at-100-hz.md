# D-36 The timer is calibrated against the 8254, and ticks at 100 Hz

Status: Accepted, 2026-08-20.

**Decision.** Calibrate the APIC timer against the 8254: counter 0, mode 0,
one twenty-millisecond window, bracketed symmetrically, divide-by-one,
rounded to nearest once from the raw pair. The result must land between 1
MHz and 50 GHz, with three attempts before the kernel boots without a clock;
a timer failure vetoes nothing else. The tick is periodic at 100 Hz. Uptime
interpolates the countdown between ticks, sandwich-read and clamped
monotonic. Calibration runs between `apic::init` and `interrupt::enable`,
unpreemptible by ordering alone. The kernel's idle becomes halt-work-halt.

**Rationale.** Nothing reports the APIC timer's bus clock rate. CPUID leaf
15H and the hypervisor leaves are absent or untrustworthy on the test
targets and the HPET sits behind the unmapped device hole of D-33; the 8254
is in every PC and hypervisor at 1.193182 MHz, over ports already safe to
drive. The sixteen-bit counter wraps ambiguously at 54.9 ms and jitter grows
as the window shrinks, so 20 ms. Channel 0's gate is always high and its
output lands on a masked controller, so no port `0x61`. The count comes from
the same divided clock the window measured, so the tick rate survives a
misfired divide write.

**Consequences.**

- The 8254 is programmed once and **parked**: a control word with no count
  halts it, and the permanent port claim stops anything restarting it. IRQ 0
  is never unmasked, sidestepping hypervisor tick coalescing.
- Accuracy is one window's worth, roughly a part in a thousand. That affects
  only long-run wall-clock drift, which nothing yet asserts.
- A critical section longer than a tick can swallow one (the APIC holds a
  single pending interrupt), and uptime then undercounts by 10 ms. Accepted
  under D-34's recorded latency cost; Phase 4's tickless design fixes it.
- 100 Hz and periodic mode are explicitly disposable when D-15 lands.

**Amended 2026-08-20.** The believability bounds miss a stall crossing the
counter's 54.9 ms span mid-window: the fold inflates the rate two to four
times, inside bounds that must span both targets' clocks. The window is
guarded in depth. No gap between counter observations may exceed ten
milliseconds. The finished window must land within one such gap of its
target. Two consecutive windows must agree to one part in sixty-four; no
stall forges the same wrong answer twice. The 8254 loads a written count
only on the crystal edge after its high byte, so the baseline read waits on
the read-back status byte's null-count flag. Attempts rose from three to
five, leaving the agreeing pair room around discarded windows.

**Superseded in part by D-57 (2026-08-23).** The tick, the interpolated
uptime and the monotonic clamp are gone. The measurement survives (the same
window, guards and agreeing pair), now applied to two clocks instead of one.
The two debts recorded here are discharged there.
