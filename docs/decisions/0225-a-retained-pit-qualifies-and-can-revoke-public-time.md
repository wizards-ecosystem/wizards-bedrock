# D-225 A retained PIT qualifies and can revoke public time

Status: Accepted, 2026-08-29. Amends D-36, D-57, D-152, D-166, D-181 and D-201; closes PW-303.

**Context.** Boot measured the timestamp counter and local-APIC timer against
two agreeing PIT windows, then parked the PIT for the rest of the boot. The
invariant-TSC bit was diagnostic only. VMware had already produced one
unexplained calibration refusal, while a rate change or stalled TSC after a
successful boot had no observer and the time page's `MODE_NONE` was only a
manually exercised protocol. A post-launch failure could therefore publish
wrong time, strand an admitted Timer, or leave a program without D-201's
preemption guarantee.

The available CPUID frequency leaves are not interchangeable. Leaf 0x15 gives
a TSC-to-crystal ratio only when denominator, numerator and crystal frequency
are all nonzero. Leaf 0x16 reports a nominal core MHz value, not necessarily
the TSC rate. Hypervisors may omit either leaf or describe a virtual frequency
which is less persuasive than a successful measurement against an independent
device.

**Decision.** PIT measurement remains the preferred boot source. When its TSC
windows refuse, a complete leaf 0x15 ratio may supply the TSC rate only if the
processor also advertises an invariant TSC; two bounded APIC windows must then
agree against that rate. Leaf 0x16 is retained for diagnostics only. The PIT
is parked on final refusal but otherwise remains claimed by the clock.

Every scheduler appointment is capped at forty milliseconds, within one
unambiguous PIT span. Arming the APIC starts a PIT window and records the TSC;
the APIC expiry samples both. A window is stable when the PIT is near the
planned expiry and its derived TSC rate is within one sixty-fourth of the boot
rate. An early APIC expiry or a TSC disagreement is conclusive. A late or
possibly folded window is inconclusive because host descheduling and slow
hardware are indistinguishable after delivery. A delivered older interrupt is
also inconclusive when the newly armed APIC counter remains nonzero; otherwise
it could accuse the replacement timer from a window only microseconds old.
Stable or inconclusive evidence breaks the failure streak; two consecutive
conclusive disagreements revoke the public clock. One was rejected after QEMU
produced a single false revocation during the first implementation boot.

Revocation records the last trusted nanosecond, freezes `now_ns`, makes the
live-calibration predicate false and publishes `MODE_NONE`. The desired mode
is stored independently of the page owner so revocation cannot be lost while
`install` temporarily moves that owner out to allocate page tables. Sleeping
threads are made runnable, their queues settle Timer operations exactly once
with `STATUS_UNAVAILABLE` when serviced, and future Timer and process admission
is refused. Kernel sleeps stop re-parking when trust is lost.

Public time and scheduling progress deliberately separate at that point. Each
later APIC delivery advances a private scheduler epoch by the simultaneous PIT
sample. The scheduler continues one-shot preemption and accounting against that
epoch without exposing it through Uptime or the time page. Idle still means no
appointment and no housekeeping tick. This validator is sampled by APIC
delivery: it detects an early APIC and a drifting or stopped TSC when an expiry
arrives, but cannot discover an APIC which never interrupts. Adding an
independently routed interrupt or NMI watchdog is a different hardware policy,
not a property inferred here.

Suspend and resume remain unsupported. There is no attempt to reconstruct an
elapsed interval across power-state discontinuity; a machine which adds that
path must requalify or replace the source before it can resume public time.

**Proof.** Host tests keep the exact leaf 0x15 ratio distinct from leaf 0x16's
nominal MHz, reject every incomplete or zero-rate answer, and distinguish
stable, stopped, early, stale-delivery and latency-ambiguous watchdog windows.
Release QEMU
injects two stopped-TSC decisions through the production streak, observes a
nondecreasing frozen public clock, a progressing private scheduler epoch,
`MODE_NONE`, and one admitted Timer's exact unavailable terminal completion;
the test then restores the live state for the remaining machine suite. The
existing page-revocation check removes the page owner while publishing
`MODE_NONE` and observes the deferred mode after restoration.

Exchanging the CPUID numerator and denominator, comparing the observed TSC
rate with itself, letting an older interrupt judge the new timer, requiring a
third disagreement, dropping the deferred page mode and suppressing private
scheduler progress each reddened only its named host or machine witness. All
six defects persist as mutations 237 through 242. The release and debug
machine suites pass 213 checks.

**Alternatives.** Promoting leaf 0x16 to a clock rate was rejected because it
names a different quantity. Preferring CPUID over a successful PIT measurement
was rejected because measured virtual hardware is the stronger local fact.
Continuing to publish the TSC after disagreement was rejected because a mode
which cannot revoke is not a trust gate. Stopping all alarms on revocation was
rejected because it recreates D-201's unpreemptible-program defect. A periodic
PIT interrupt was rejected because it would add routing and a permanent tick;
HPET or ACPI PM-timer support remains justified only by a machine which needs
it. Panicking on the first disagreement was rejected after the real QEMU false
positive demonstrated the cost.

**Consequences.** Long appointments use forty-millisecond validation steps;
the two-hundred-millisecond sleep witness therefore expects five useful
expiries rather than one. The page's conversion remains immutable, while its
permission is now a live production state. The machine-check count is 213 and
the mutation registry is 242. PW-301 and PW-003 still own the current VMware
observation; this decision does not upgrade QEMU's CPUID or APIC evidence into
one.
