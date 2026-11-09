# D-359 The storage candidate is sequenced through sibling stages, so one stage frame at a time sits on the boot stack

Status: Accepted (2026-10-03), amends D-287 and D-358; closes PW-572

**Context.** Storage selection runs on the boot thread's guarded 32 KiB stack
with interrupts off, and the debug build gives every by-value owner and every
temporary its own slot (D-287). D-287 split NVMe preparation into stages and
D-358 moved the remapping unit into a kernel-owned slot, but nothing bounded the
sequencer itself. A static model of the debug ELF, summing each prologue's
reservation along the deepest call path, put candidate 1's retirement path at
32,024 of 32,768 bytes (744 free) and candidate 2's start path at 30,944 bytes:
`try_controller` reserved 7,696 bytes and the candidate finish, merged back into
one function in defa419, 9,360 bytes, and both stayed live above
`retire_candidate` and the lease release beneath it. The 2026-10-01 debug boots
had double-faulted with a slightly deeper nesting, and today's passed only
because the real figure sat just under the guard. The painted stack meter of the
next decision needs a structure that keeps two pages free before it can require
them.

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
   after the VT-d frame returns. `try_controller` falls from 7,696 to about
   2.7 KiB.
3. *The shape is a contract.* `dma_contracts::only_the_candidate_sequencer_calls_its_stages`
   requires that every function returning `CandidateStage` which
   `try_controller` calls is called exactly once and only from
   `try_controller`, that the borrowing isolation stage and `retire_candidate`
   are called the same way, and that no stage calls another. The self-test observer contract's `CandidateBaseline`
   count falls from five to four, because the baseline is captured and checked
   in the sequencer alone. Retirement and quarantine arms keep their counts.

**Evidence.** On 2026-10-03 the deepest static debug paths fell from 32,024
bytes (retirement) and 30,944 (`nvme::start`) to 17,560 and 24,208, and every
path through `claim_candidate`, `admit_candidate`, `start_candidate`,
`iommu::establish`, `nvme::prepare` and `identify_namespace` now leaves at
least 8,560 bytes free; the measurements are the prologue sums of the debug
self-test ELF before and after. The debug and release self-tests passed 270/270,
`boot-check` and both `degraded-check` boots settled, and every pinned log line
is unchanged. Mutation 1,570 runs the admit stage inside the claim stage's frame
and reds the sequencer contract.

**Alternatives.** A larger boot stack was rejected: it moves the guard without
bounding growth, and D-287 and D-326 already refuse it. Nesting the stages,
each calling the next as a tail call, was rejected because both frames stay
live, which is the shape that double-faulted. Boxing the owners between stages
was rejected because a box is a heap allocation inside the candidate window
and the resource baseline would halt selection (D-199). Measuring without
restructuring was rejected because a meter over the whole candidate would have
failed the two-page rule by six to seven kilobytes in debug.
