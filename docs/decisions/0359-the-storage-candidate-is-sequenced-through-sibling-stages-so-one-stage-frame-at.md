# D-359 The storage candidate is sequenced through sibling stages, so one stage frame at a time sits on the boot stack

Status: Accepted, 2026-10-03. Amends D-287 and D-358; closes PW-572.

**Decision.**

1. *A thin sequencer.* `try_controller` captures its self-test baseline, calls
   `claim_candidate` (lease, register window and `nvme::prepare`), then invokes
   the borrowing `isolate_candidate` before it loops through `admit_candidate`
   (bus-master admission) and `start_candidate` (start, block 0, commit) until
   the candidate is settled or handed back for retirement, and calls
   `retire_candidate` itself. Each stage is a sibling call that returns before
   the next begins, so one stage frame at a time sits above the sequencer. The
   isolation split was added with D-370 when the complete requester inventory
   made nesting `iommu::establish` inside the owner-moving claim stage exceed
   the measured margin in debug.
2. *One copy of the owners.* The stages pass a `CandidateStage` (`Prepared`,
   `Admitted`, `Retire`, `Settled`) by `&mut` and take their owners out of it,
   so the sequencer's frame holds the candidate once rather than once per
   stage call. Isolation borrows the prepared device and changes only its stage
   flag on success; an uncertain result moves the owners into quarantine only
   after the VT-d frame returns.
3. *The shape is a contract.* `dma_contracts::only_the_candidate_sequencer_calls_its_stages`
   requires that every function returning `CandidateStage` which
   `try_controller` calls is called exactly once and only from
   `try_controller`, that the borrowing isolation stage and `retire_candidate`
   are called the same way, and that no stage calls another. The self-test
   observer contract's `CandidateBaseline` count falls from five to four,
   because the baseline is captured and checked in the sequencer alone. Retirement and quarantine
   arms keep their counts.

**Rationale.** Storage selection runs on the boot thread's guarded 32 KiB stack
with interrupts off, and the debug build gives every by-value owner and
temporary its own slot (D-287). Nothing bounded the sequencer itself: with the
merged candidate finish live above `retire_candidate`, the deepest debug path
sat just under the guard and slightly deeper nesting had double-faulted, so the
painted stack meter needed a structure that keeps two pages free before it
could require them. On 2026-10-03 the static debug model left at least 8,560
bytes free on every candidate path, and mutation 1,570, which runs the admit
stage inside the claim stage's frame, reds the sequencer contract.

**Alternatives.** A larger boot stack was rejected: it moves the guard without
bounding growth, and D-287 and D-326 already refuse it. Nesting the stages,
each calling the next as a tail call, was rejected because both frames stay
live, which is the shape that double-faulted. Boxing the owners between stages
was rejected because a box is a heap allocation inside the candidate window
and the resource baseline would halt selection (D-199). Measuring without
restructuring was rejected because a meter over the whole candidate would have
failed the two-page rule by six to seven kilobytes in debug.
