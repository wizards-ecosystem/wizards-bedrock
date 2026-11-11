# D-240 A round a human answers is bounded by their clock, and prompted on their screen

Status: Accepted, 2026-08-29. Amends D-18, D-135 and D-145; advances PW-003.

**Decision.** A wait whose answer comes from a person is bounded by the clock,
not by iterations. `PollLimit`, written for NVMe's polls in D-200, moves to
`libs/timebase` and both callers share it; the spin count stays as what it
always claimed to be, the fallback for a clock that is not running. One
constant, `HUMAN_WAIT_MS`, gives every human round the same forty-five seconds,
so the prompt and the measurement beside it cannot disagree.

A prompt for a person is written where the person is looking. `awaiting` logs
the prompt verbatim, so the harness matches it and the transcript records it,
and also writes the instruction to the framebuffer, in the register `panic`
already uses for the other thing someone has to be told.

**Rationale.** After D-239 fixed the decoder, PW-003's human round still
failed. The keystroke wait was two million spins, a fraction of a second on a
3.4 GHz machine, so its ten-second deadline was never consulted, though its own
doc described a clock deadline with an iteration cap only for a dead clock; it
was the third comment stating a bound the code did not hold that month, after
`MAX_CHAIN` (D-238) and the identity arbiter (D-239). The storage-delay round
sampled over one second, sized for QMP. And `log.rs` sends records to serial
alone, so the three `awaiting …` prompts were never rendered on any screen the
person D-18 requires had. A check whose subject cannot see what it is being asked is a defect
in the evidence path, not a cosmetic one.

**Alternatives.** Keeping the iteration cap as the primary bound was rejected
because it makes the wait a function of processor speed, which is what hid this.
Duplicating `PollLimit` rather than moving it was rejected: two copies of one
rule is how the wait came to disagree with the measurement beside it. Routing
every log record to the framebuffer was rejected because D-135's reason holds,
a diagnostic must not scroll the banner away, so only the three prompts cross.
Automating the round was rejected again for D-235's reason: Workstation exposes
no bounded, owned input channel.

**Consequences.** On VMware every keyboard check passed and the delayed read
evidence read `interrupts=true`. `the_human_wait_fits_the_harness_patience`
reads `HUMAN_WAIT_MS` from the kernel and fails if three unanswered rounds
could outlast the harness, so an unanswered round stays the kernel's red check
rather than a killed emulator; mutation 290 keeps it red. The three moved
`PollLimit` mutations name `libs/timebase`. `a later nvme controller survives
an earlier failure` failed there because the VMware machine had one NVMe
controller where the check asks for a useless candidate and a usable sibling,
which QEMU has offered since D-199; the template now offers both, and mutation
291 keeps the machine-shape test red. That template change was unbooted when
this was written and is not claimed as evidence.
