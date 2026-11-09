# D-57 The tick is retired: the counter is the clock, the timer is an alarm

Status: Accepted (2026-08-23)

**Context.** D-15 wanted tickless from the start and D-36 built a 100 Hz
periodic tick anyway, calling it "explicitly disposable when D-15 lands" —
because a tickless kernel decides when next to wake, and that decision
belongs to a scheduler. Slice 2 built the scheduler. D-36's tick also owed
two debts: a critical section longer than a tick swallows one, and uptime
interpolates the countdown the tick reloads, so retiring the tick deletes
uptime's denominator along with it.

**Options for the timebase.**

- *Accumulated deadlines.* Sum each one-shot interval as it expires, plus
  where the countdown now stands. Needs no new hardware, and has a hole
  exactly where tickless puts one: while nothing is armed, nothing accrues,
  so an idle machine's clock stops. Closing it means arming a housekeeping
  timer, which is the tick under another name.
- *The HPET or the ACPI power-management timer.* Real free-running counters,
  both reached through tables and mappings this kernel does not yet parse,
  and both an order of magnitude slower to read than a register.
- *TSC-deadline mode.* Arms the timer from a comparison against the counter
  rather than from a countdown. A different interface, not a different
  timebase, and it needs everything below first.
- *The timestamp counter.* Free-running, one instruction to read, and its
  rate is measurable by exactly the machinery D-36 already built — the same
  twenty-millisecond window, with two more reads inside it.

**Decision.** Two clocks, measured in one window, each doing the job it is
shaped for. The timestamp counter is the clock: `now_ns` is a subtraction and
a multiply. The APIC timer is the alarm: one-shot, armed for the next moment
the scheduler has a reason to look — the running thread's slice ending, or
the earliest sleeper's deadline — and **stopped** when there is neither.
Every path that can change that answer ends in the same call, so the arming
rule lives in one place.

The conversions are `libs/timebase`: a rounded multiply-and-shift, thirteen
host tests. The multiplier is held to thirty-two bits, which buys two things
— any `u64` times it fits a `u128`, and a whole conversion packs into one
word, which is how the clock is published to a reader that cannot take a lock
because it is inside an interrupt handler.

**Rationale.** A counter that free-runs is the only thing that makes an idle
machine's clock keep time, and an idle machine's clock keeping time is the
whole difference between tickless and a tick with extra steps. Reading it is
a multiply because the read path is every scheduling decision; a sixty-four
bit division there would be paid for thousands of times a second.

**Consequences.**

- **The monotonic clamp is deleted, and that makes its check mean
  something.** D-36 clamped every reading against a high-water mark because a
  reader with the flag clear could watch the countdown reload while the tick
  it announced stayed pending. There is no countdown to reload now, so
  monotonicity comes from the counter, and "uptime never runs backwards" has
  stopped being a check that could not fail. Breaking it — one exclusive-or
  on the low byte of every reading — turned it red for the first time.
- **The invariant-counter bit is reported, not obeyed.** `cpuid` leaf
  0x8000_0007 is the architectural promise that the counter runs at a
  constant rate, and a machine without it is warned about by name at boot. It
  is not a veto: QEMU under TCG refuses to advertise the bit — the gate's own
  machine — while providing a perfectly steady counter, and the evidence this
  kernel actually has is two independent windows that agreed to one part in
  sixty-four. Refusing to boot on the promise while holding the evidence
  would be the wrong way round. VMware advertises it, which is where the
  phase's D-18 boot observes it true.
- **Time is nanoseconds now, and the slice is three milliseconds** — a tenth
  of what D-53 could afford, because the cost of a short slice used to be a
  tick that could not resolve one and is now a timer armed for exactly it.
  D-53's coarse-attribution debt is paid: stints are charged in nanoseconds.
- Two clamps bound what may be asked of the alarm. Below, fifty microseconds:
  a deadline already passed would otherwise arm one tick of a gigahertz clock
  and take the interrupt again immediately. Above, the counter's thirty-two
  bits — about four seconds at a gigahertz — after which the expiry re-arms
  for the remainder rather than being handed a wrapped count.
- The debt D-36 recorded is discharged rather than reduced. A deadline that
  passes while the flag is clear is delivered late, not lost, and uptime does
  not undercount because it never depended on the interrupt.
- Deleted with the tick: `x86_apic::timer::elapsed_milliseconds` and its
  interpolation tests, `x86_pit::calibrate::periodic_initial_count`,
  `apic::start_periodic_timer`, `time::ticks`, `MS_PER_TICK`, and the two
  self-checks that counted ticks. `calibrate::divided_hz` is now
  `measured_hz`, because one window measures two clocks.
- Six checks on the machine, each broken once. The one that matters is **an
  idle kernel takes no timer interrupts** — not fewer, none — against the
  fifty the retired tick would have taken in the same half second. Its
  counterpart under load bounds the same counter from both sides, and is also
  what proves the handler's end-of-interrupt, which the retired tick's pair of
  checks used to.
- Two of the breaks hit the wrong path first, which is now three times this
  phase. Making the measured counter rate a thousand times too fast turned the
  machine into an interrupt storm — a three-microsecond slice — before the
  check that reads the rate ever ran. And breaking the expiry handler's
  re-arming could not redden a check about an *idle* kernel, which by
  definition never enters it; the break that reaches it is one to the call
  that stops the timer.
