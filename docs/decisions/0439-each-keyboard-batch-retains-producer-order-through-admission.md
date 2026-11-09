# D-439 Each keyboard batch retains producer order through admission

Status: Accepted (2026-10-23), qualification OPEN; advances PW-720

The debug boot in `pristine-dg4d95uw` completed 293 of 294 checks and failed
the blocked reader's expected A key, while the armed A, editing and translated
key rounds passed. Its log does not identify the failing reader conjunct.
Review found a concrete ordering gap: a caller released the keyboard lock
after decoding a batch, then enqueued that batch without excluding another
polling producer. Preemption there could enqueue later keys before the first
batch. Both the permanent poller and the self-test's direct wait can call this
production path. This is a supported failure path, not proof that the captured
boot took it.

One shared batch producer excludes interrupts from drain through ordered
delivery, with all existing locks taken sequentially. Its existing limits
remain eight events per batch and sixteen controller bytes per event.
Exclusion is restored after each batch; reader wakeup occurs outside every
batch's exclusion and lock. This is the current single-CPU contract, not an
SMP synchronization claim. Holding a keyboard leaf guard while acquiring the
event lock was rejected because leaf guards cannot nest.

The self-test arrival watch also advances only after queue admission succeeds.
Previously it observed even dropped events, contradicting the wait's claim.
Admission and observation use one shared delivery composition, keeping its
locks sequential. A successful watch remains historical observation, never a
reservation against another consumer.

The existing input witness exercises the exact batch producer with finite
local batches and a competing producer at the drain-to-delivery boundary,
conditional on the actual CPU interrupt flag. It proves that the competing
batch cannot overtake the first and that exclusion is restored before its
deferred delivery. This runtime interleaving fixture does not claim delivery
of a hardware interrupt; the live injected-key and blocked-reader checks
remain separate evidence. Shared delivery probes fill the queue and require
refused press/release events not to advance the watch, followed by an admitted
pair that does. Retained removal-of-exclusion and observe-refused-event
defects must reach the exact completed machine red, then restore.

No new machine check count, polling deadline or input capacity is introduced.
Current paired QEMU, fresh VMware, ordinary CI and the complete mutation sweep
remain required before qualification closes.
