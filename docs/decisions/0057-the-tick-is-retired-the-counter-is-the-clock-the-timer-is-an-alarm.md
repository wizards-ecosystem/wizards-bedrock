# D-57 The tick is retired: the counter is the clock, the timer is an alarm

Status: Accepted, 2026-08-23.

**Context.** D-15 wanted tickless from the start; D-36 built a 100 Hz
periodic tick anyway, calling it "explicitly disposable when D-15 lands". A
tickless kernel decides when next to wake, which is a scheduler's call, and
slice 2 built the scheduler. D-36's tick also owed two debts. A critical
section longer than a tick swallows one. Uptime interpolates the countdown
the tick reloads, so retiring the tick deletes uptime's denominator too.

**Options for the timebase.**

- *Accumulated deadlines.* Sum each one-shot interval as it expires, plus
  where the countdown now stands. No new hardware, but a hole where
  tickless puts one: while nothing is armed, nothing accrues, and an idle
  machine's clock stops. Closing it means a housekeeping timer, which is
  the tick under another name.
- *The HPET or the ACPI power-management timer.* Free-running counters,
  both behind tables and mappings this kernel does not yet parse, and both
  an order of magnitude slower to read than a register.
- *TSC-deadline mode.* Arms the timer by comparison against the counter
  instead of a countdown. A different interface, not a different timebase,
  and it needs everything below first.
- *The timestamp counter.* Free-running, one instruction to read, with a
  rate measurable by the machinery D-36 already built: the same
  twenty-millisecond window, with two more reads inside it.

**Decision.** Two clocks, measured in one window, each doing the job it is
shaped for. The timestamp counter is the clock: `now_ns` is a subtraction
and a multiply. The APIC timer is the alarm. It is one-shot, armed for the
next moment the scheduler has a reason to look (the running thread's slice
ending, or the earliest sleeper's deadline), and **stopped** when there is
neither. Every path that can change that answer ends in the same call, so
the arming rule lives in one place.

The conversions are `libs/timebase`: a rounded multiply-and-shift, thirteen
host tests. The multiplier is held to thirty-two bits. Any `u64` times it
then fits a `u128`, and a whole conversion packs into one word. That word
publishes the clock to a reader inside an interrupt handler, which cannot
take a lock.

**Rationale.** Only a free-running counter keeps an idle machine's clock on
time, and that separates tickless from a tick with extra steps. Reading it
is a multiply because the read path is every scheduling decision; a
sixty-four bit division there would be paid thousands of times a second.

**Consequences.**

- **The monotonic clamp is deleted, and that makes its check mean
  something.** D-36 clamped every reading against a high-water mark: a
  reader with the flag clear could see the countdown reload while the tick
  it announced stayed pending. No countdown reloads now; monotonicity comes
  from the counter. "Uptime never runs backwards" is no longer a check that
  cannot fail. Breaking it (one exclusive-or on the low byte of every
  reading) turned it red for the first time.
- **The invariant-counter bit is reported, not obeyed.** `cpuid` leaf
  0x8000_0007 promises a constant counter rate, and a machine without it is
  warned about by name at boot. It is not a veto. QEMU under TCG, the
  gate's own machine, does not advertise the bit yet provides a steady
  counter. This kernel's evidence is two independent windows that agreed to
  one part in sixty-four. Refusing to boot on the promise while holding the
  evidence would be backwards. VMware advertises it, so the phase's D-18
  boot observes it true.
- **Time is nanoseconds now, and the slice is three milliseconds**, a tenth
  of what D-53 could afford. A short slice once cost a tick that could not
  resolve it; now it costs a timer armed for it. D-53's coarse-attribution
  debt is paid: stints are charged in nanoseconds.
- Two clamps bound what the alarm may be asked. Below, fifty microseconds:
  a deadline already passed would otherwise arm one tick of a gigahertz
  clock and take the interrupt again at once. Above, the counter's
  thirty-two bits (about four seconds at a gigahertz); past that the expiry
  re-arms for the remainder instead of taking a wrapped count.
- The debt D-36 recorded is discharged, not reduced. A deadline passing
  while the flag is clear arrives late, not lost, and uptime cannot
  undercount because it never depended on the interrupt.
- Deleted with the tick: `x86_apic::timer::elapsed_milliseconds` and its
  interpolation tests, `x86_pit::calibrate::periodic_initial_count`,
  `apic::start_periodic_timer`, `time::ticks`, `MS_PER_TICK`, and the two
  self-checks that counted ticks. `calibrate::divided_hz` is now
  `measured_hz`, because one window measures two clocks.
- Six checks on the machine, each broken once. The key one: **an idle
  kernel takes no timer interrupts**, none, against the fifty the retired
  tick would take in the same half second. Its counterpart under load
  bounds the same counter from both sides. It also proves the handler's
  end-of-interrupt, as the retired tick's pair of checks did.
- Two breaks hit the wrong path first, now three times this phase. A
  measured counter rate a thousand times too fast made an interrupt storm
  (a three-microsecond slice) before the rate check ran. Breaking the
  expiry handler's re-arming could not redden a check on an *idle* kernel,
  which never enters it; the break that reaches it targets the call that
  stops the timer.
