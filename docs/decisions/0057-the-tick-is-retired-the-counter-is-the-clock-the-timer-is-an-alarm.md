# D-57 The tick is retired: the counter is the clock, the timer is an alarm

Status: Accepted, 2026-08-23.

**Decision.** Two clocks, measured in one window, each doing the job it is
shaped for. The timestamp counter is the clock: `now_ns` is a subtraction
and a multiply. The APIC timer is the alarm. It is one-shot, armed for the
next moment the scheduler has a reason to look (the running thread's slice
ending, or the earliest sleeper's deadline), and **stopped** when there is
neither. Every path that can change that answer ends in the same call, so
the arming rule lives in one place.

The conversions are `libs/timebase`: a rounded multiply-and-shift. The
multiplier is held to thirty-two bits. Any `u64` times it then fits a
`u128`, and a whole conversion packs into one word. That word publishes the
clock to a reader inside an interrupt handler, which cannot take a lock.

**Rationale.** Only a free-running counter keeps an idle machine's clock on
time, and that separates tickless from a tick with extra steps: summing
one-shot intervals stops the clock while nothing is armed, and the HPET and
ACPI timer sit behind unparsed tables and read far slower. The TSC's rate is
measured by D-36's window. Reading it is a multiply because the read path is
every scheduling decision; a sixty-four bit division there would be paid
thousands of times a second.

**Consequences.**

- **The monotonic clamp is deleted.** No countdown reloads now; monotonicity
  comes from the counter, so "uptime never runs backwards" is a check that
  can fail.
- **The invariant-counter bit is reported, not obeyed.** `cpuid` leaf
  0x8000_0007 promises a constant counter rate, and a machine without it is
  warned about by name at boot. It is not a veto: QEMU under TCG does not
  advertise the bit yet provides a steady counter, and this kernel's
  evidence is two independent windows that agreed to one part in sixty-four.
- **Time is nanoseconds, and the slice is three milliseconds**: a short
  slice now costs a timer armed for it, not a tick that cannot resolve it.
  Stints are charged in nanoseconds, replacing D-53's millisecond
  attribution.
- Two clamps bound what the alarm may be asked. Below, fifty microseconds: a
  deadline already passed would otherwise arm one tick of a gigahertz clock
  and take the interrupt again at once. Above, the counter's thirty-two bits
  (about four seconds at a gigahertz); past that the expiry re-arms for the
  remainder instead of taking a wrapped count.
- The debt D-36 recorded is discharged, not reduced. A deadline passing
  while the flag is clear arrives late, not lost, and uptime cannot
  undercount because it never depended on the interrupt.
- **An idle kernel takes no timer interrupts**, and a self-check holds it to
  none.
