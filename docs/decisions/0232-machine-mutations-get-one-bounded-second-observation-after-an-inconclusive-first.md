# D-232 Machine mutations get one bounded second observation after an inconclusive first

Status: Accepted, 2026-08-29. Advances PW-001.

**Decision.** A machine mutation receives exactly one second observation only
when its first observation is inconclusive. An exact catch and a clean survivor
are terminal results and are never sampled until they change. A second
inconclusive observation fails qualification and reports both causes. Build,
boot and observation deadlines remain the existing bounded ones; there is no
open-ended retry.

The forced NVMe timeout selects a zero-attempt `PollLimit` and falls through
the ordinary production timeout arm, so its self-test observes the same
`fail_submitted` cleanup a real expired command uses. The stale queue mutation
names `PendingTable::slots` explicitly again, preserving its original compiled
ownership-loss defect.

**Rationale.** The first closing sweep of the 276-entry registry left six
entries inconclusive: one no longer compiled, the NVMe timeout seam returned
through a self-test-only arm before the production cleanup the mutation
removed, and four reached an unrelated verdict or a common page fault, then
their exact witness when rerun unchanged. Treating that incidental first
observation as causal would violate D-209; treating it as permanent would
discard a valid, repeatable D-28 witness.

**Alternatives.** Crediting any red verdict was rejected because it recreates
the pre-D-134 causal hole. Retrying survivors was rejected because a green
mutant is already a valid negative result and repeated sampling could hide a
nondeterministic proof gap. Unlimited retry was rejected because qualification
must terminate. Keeping the NVMe shortcut and retargeting the mutation to test
code was rejected because it would prove a seam instead of the production
cleanup arm.

**Consequences.** Removing the NVMe terminal cleanup fails only `an nvme
timeout quiesces before dma memory returns`, and the check repairs the
deliberately live DMA state before continuing. Mutation 277 keeps
`an_inconclusive_machine_observation_gets_one_bounded_retry` red when the bound
drops from two to one. The complete successor sweep remains PW-001 work and
none of the failed 276-entry attempt is promoted into closing evidence.
