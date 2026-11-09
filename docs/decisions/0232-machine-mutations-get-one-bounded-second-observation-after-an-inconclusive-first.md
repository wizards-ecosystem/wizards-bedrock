# D-232 Machine mutations get one bounded second observation after an inconclusive first

Status: Accepted (2026-08-29), advances PW-001

**Context.** The first closing sweep of the 276-entry registry caught 270
mutations through the exact witness each named and found no survivor. Six
entries were inconclusive. One queue mutation no longer compiled after
`PendingTable` encapsulated its slots. The NVMe timeout seam returned through a
self-test-only arm before reaching the production terminal cleanup the
mutation removed. The other four entries reached an unrelated verdict or one
common page fault during the sweep, then reached their exact panic or named
check when rerun unchanged. Treating that incidental first observation as
causal would violate D-209; treating one incidental observation as permanent
would discard a valid, repeatable D-28 witness.

**Decision.** A machine mutation receives exactly one second observation only
when its first observation is inconclusive. An exact catch and a clean survivor
are terminal results and are never sampled until they change. A second
inconclusive observation fails qualification and reports both causes. Build,
boot and observation deadlines remain the existing bounded ones; there is no
open-ended retry.

The forced NVMe timeout now selects a zero-attempt `PollLimit` and falls through
the ordinary production timeout arm. Its self-test therefore observes the
same `fail_submitted` cleanup a real expired command uses. The stale queue
mutation names `PendingTable::slots` explicitly again, preserving its original
compiled ownership-loss defect.

**Alternatives.** Crediting any red verdict was rejected because it recreates
the pre-D-134 causal hole. Retrying survivors was rejected because a green
mutant is already a valid negative result and repeated sampling could hide a
nondeterministic proof gap. Unlimited retry was rejected because qualification
must terminate. Keeping the NVMe shortcut and retargeting the mutation to test
code was rejected because it would prove a seam instead of the production
cleanup arm.

**Consequences.** The repaired NVMe path passed all 215 checks; removing its
terminal cleanup then failed only `an nvme timeout quiesces before dma memory
returns` at 214/215 and the check repaired the deliberately live DMA state
before continuing. The corrected queue mutation compiled and failed its named
teardown check. Unchanged reruns of the trap-context, refused-arrangement,
refused-settlement and unguarded-stack mutations reached their exact expected
outcomes. Reducing the observation bound from two to one failed only
`an_inconclusive_machine_observation_gets_one_bounded_retry`; mutation 277
retains that defect. The complete successor sweep remains PW-001 work and none
of the failed 276-entry attempt is promoted into closing evidence. No unsafe
operation, wire ABI or machine-check count changed.
