# D-240 A round a human answers is bounded by their clock, and prompted on their screen

Status: Accepted, 2026-08-29. Amends D-18, D-135 and D-145; advances PW-003.

**Context.** D-239 fixed the decoder and PW-003's human round still failed. Four
more VMware boots found four defects, none of them in the kernel's behaviour and
all of them in what the round asks of the person D-18 requires.

The keystroke wait was `for _ in 0..WAIT_SPINS`, two million iterations of a
lock and a status read. On a 3.4 GHz machine that is a fraction of a second, so
the ten-second deadline beside it was never once consulted and the prompt
scrolled past in the same burst as everything around it. Its own doc already
described the rule it did not implement — "a deadline on the clock for the
honest case, and an iteration cap so a dead clock converts the wait into a red
check rather than a hang" — which is the third comment stating a bound the code
did not hold this month, after `MAX_CHAIN` (D-238) and the identity arbiter
(D-239). Raising the deadline to forty-five seconds changed nothing, because the
spin count was the real bound.

Beside it, the storage-delay round sampled interrupt progress over one second
after printing its prompt. Ten seconds and one second were both sized for QMP,
which injects in microseconds; a person reading an instruction meets neither. A
prompt that waits one length and a measurement that samples another is how a
round comes to be unanswerable in two independent ways.

And `log.rs` sends records to the serial port alone, so that a diagnostic never
scrolls the banner away. The three `awaiting …` prompts are log records. The
human was being asked to answer prompts that were never rendered on any
screen they had. The round was not merely unlit: it was **unobservable**,
and a check whose subject cannot see what it is being asked is a defect in
the evidence path, not a cosmetic one. PW-003 requires a human transcript,
and the human could not read the question.
Five boots were spent on instructions to watch for text that did not exist, and
the operator was right each time they said they had never seen it.

**Decision.** A wait whose answer comes from a person is bounded by the clock,
not by iterations. `PollLimit` — written for NVMe's polls in D-200, whose own
doc says a monotonic limit does not spend attempts — moves to `libs/timebase`
and both callers share it; the spin count stays as what it always claimed to be,
the fallback for a clock that is not running. One constant, `HUMAN_WAIT_MS`,
gives every human round the same forty-five seconds, so the prompt and the
measurement beside it cannot disagree.

A prompt for a person is written where the person is looking. `awaiting` logs
the marker verbatim — the harness matches it and the transcript records it — and
also writes the instruction to the framebuffer, in the register `panic` already
uses for the other thing someone has to be told.

**Alternatives.** Keeping the iteration cap as the primary bound was rejected
because it makes the wait a function of processor speed, which is what hid this.
Duplicating `PollLimit` rather than moving it was rejected: two copies of one
rule is how the wait came to disagree with the measurement beside it. Routing
every log record to the framebuffer was rejected — D-135's reason holds, a
diagnostic must not scroll the banner away — so only the three prompts cross.
Automating the round was rejected again for D-235's reason: Workstation exposes
no bounded, owned input channel.

**Consequences.** The round is answerable. On VMware, `passed=214 failed=1`
against `passed=208 failed=7` before D-239: every keyboard check green, and
`delayed read evidence … interrupts=true` where five previous transcripts read
`false`. `the_human_wait_fits_the_harness_patience` reads `HUMAN_WAIT_MS` from
the kernel and fails if three unanswered rounds could outlast the harness, so an
unanswered round stays the kernel's red check rather than a killed emulator;
doubling the constant fails it, and mutation 290 retains that. The three moved
`PollLimit` mutations now name `libs/timebase`. The remaining VMware failure,
`a later nvme controller survives an earlier failure`, was not a keyboard
defect and not answerable by any keypress: the VMware machine had one NVMe
controller where the check asks for a useless candidate and a usable
sibling, which QEMU has offered since D-199. The template now offers both,
and restoring a namespace to the first fails only the machine-shape test;
mutation 291 retains it. That change is unbooted at this writing and is
not claimed as evidence.
No unsafe operation or wire ABI changed, and the machine-check count is
unchanged at 215.
