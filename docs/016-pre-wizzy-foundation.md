# 016 - The finite pre-Wizzy kernel foundation register

This is the handoff and the only current backlog for work in the Wizard's OS
repository while the Wizzy compiler-readiness gate is closed. D-183 remains
the language and sequencing authority. D-184 makes this register finite and
prevents "kernel polish" from becoming an excuse to design the userland before
its real compiler artifact exists.

The inventory was made on 2026-08-26 from `main` at `853f967`, after the Phase
6 slices had landed. It includes live-code inspection, every explicit "left
owed" item in the decision log and phase documents, and the outstanding
verification evidence. A later discovery may be added only with a concrete
defect, violated invariant, missing proof, or named Wizzy compiler handoff
consumer. "State of the art", "useful eventually", and "nice to have" are not
admission reasons.

## What completion means

The planned Phase 6 slices are landed, but the phase and polished foundation
are **not closed**. `014-storage.md`'s closing table owns the checked current
QEMU and mutation totals. The qualification history is worth keeping, because
each attempt found the next defect rather than the same one: the first
276-entry closing attempt caught 270 exactly and exposed six qualification
defects; D-232 repaired the direct ones; its successor exposed cross-mutation
build contamination, which D-233 diagnosed; the complete 278-entry successor
caught 274 exactly with no survivor and exposed the four observer/order gaps
D-234 repairs; and the 283-entry successor exposed the contamination D-233 and
D-236 had each half-repaired. D-237 replaced all of that with its cause — a
restored source must be written forward in time, not copied backwards — and
the complete 285-entry sweep on `ff6ddd3` then caught every entry through the
check it names, with no survivor and no second observation spent anywhere.

What is left is the human VMware round. The current image has not been booted
by a person, so PW-003, PW-004 and the three observations which depend on them
are open, and the phase does not close until they are. The compiler gate in
`015-wizzy-userland.md` is also **CLOSED**, and closing Phase 6 does not open
it.

The kernel is ready for the first real Wizzy integration only when:

1. every `REQUIRED`, `READY`, `VERIFY`, and `DECIDE` row below is changed to
   `DONE` or `SUPERSEDED`, with its proof or superseding ADR linked in the row;
2. the phase-close evidence names one exact commit and records CI, the full
   current mutation sweep, model checking, the self-test VMware image, and the
   shipped VMware image;
3. the five P0 safe-boundary/device defects found by this audit are closed;
4. all four `WZ-*` compiler-readiness rows are complete together at one
   reachable, pinned `wizards-ink` revision; and
5. a gate-opening ADR records the pins, commands, hashes, runtime seam and
   handoff version. It does not infer them from a sibling checkout.

`TRIGGER` is not unfinished work. It means **do not build this yet**; the row
records the exact event that can move it into the live register. Post-gate and
later-phase product work is classified at the end and is not part of kernel
completion.

## Register vocabulary

| State | Meaning |
|---|---|
| `REQUIRED` | Evidence required to close the current phase; run it on the final commit |
| `READY` | A confirmed defect or finite proof/tooling debt which D-183 permits now |
| `VERIFY` | A credible audit finding; first reproduce or disprove it, then fix or close it with that evidence |
| `DECIDE` | Current evidence forces a policy or architecture choice; write the ADR before code |
| `WIZZY` | Work belongs in `wizards-ink`, not as replacement OS code |
| `TRIGGER` | Deliberately stopped until the named caller or hardware/concurrency condition exists |
| `DONE` | Deliverable and red/green proof are recorded |
| `SUPERSEDED` | A later ADR explicitly removes or replaces the obligation |

P0 is a safe-abstraction, memory-safety, DMA, privilege, or device-ownership
boundary. P1 is correctness, liveness, deterministic cleanup, or resilience.
P2 is qualification, reproducibility, maintainability, and evidence quality.

<!-- PRE-WIZZY-REGISTER:START -->

## Phase-close evidence

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-001 | DONE | P0 | The 278-entry diagnostic successor caught 274 exactly with no survivor; D-234 repaired its observer/order gaps, and the first 283-entry successor traced its remaining contamination to a restored file's timestamp, which D-237 fixed at its cause | Run `cargo xtask mutate` from a clean tree on the closing commit; preserve per-mutation causality | 2026-08-29 the complete 285-entry sweep on clean commit `ff6ddd3` caught every mutation through the check it names in 44 minutes, with no survivor and no inconclusive first observation anywhere — the first sweep to spend no second observation at all; the journal recorded source `29e66024…5e283e11` and index `29423478…9474e028`, and the tree was byte-identical afterwards |
| PW-002 | DONE | P0 | The ring model is a phase-close gate outside CI | Re-run `cargo xtask model-check` on the closing commit | 2026-08-29 OpenJDK 21.0.5 and TLC 2026.08.21.155922 completed `Ring` at 2,901 generated and 1,040 distinct states to depth 49, and `RingMemory` at 2,809 and 1,296 to depth 31, both with zero errors on `ff6ddd3`; all four model mutations violated only the invariant each names |
| PW-003 | REQUIRED | P0 | The dated VMware result predates D-181/D-182 and covered 182/187, not the current image | Generate `cargo xtask vmware`, boot its identified self-test image and perform every human keyboard round | A dated zero-failure fresh transcript at the verdict's expected total, including image digest, WP, SMEP/SMAP/UMIP/NX, compacted XSAVE, x2APIC, calibrated time, user launch, storage and input evidence |
| PW-004 | REQUIRED | P1 | A self-test image is not the image users boot | Generate `cargo xtask vmware --shipped` and boot its separately identified machine after PW-003 | Its fresh transcript reaches its prompt, storage/VFS survey and user boundary without panic; record image digest and VM/tool version |
| PW-005 | REQUIRED | P1 | The ordinary gate must describe the same commit as the exceptional evidence | Run `cargo xtask ci` after every register-closing change and once after the evidence-only close | Release/debug self-tests, shipped liveness and degraded boot all pass on the recorded commit |
| PW-006 | REQUIRED | P1 | D-181 and D-182 were post-audit corrections, and this audit found deeper safe/device boundaries | Re-attack hostile inputs, cleanup paths, safe `wost` callers and release-only invariant failures after the P0/P1 fixes | Phase-close ADR links the audit, every break-test, full evidence manifest and remaining trigger-only rows; no historical evidence is invented |

## Safe memory, paging and resource lifetime

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-100 | DONE | P0 | Safe public `AddressSpace::map_shared` admitted that `Shared` could die while the mapping survived, leaving a user alias to recycled frames | D-185 makes the space retain a non-accessing allocation keepalive until proven reclaim | 2026-08-27 QEMU `a shared mapping keeps its frame` held the frame after the owner dropped and released it only after the mapping became unreachable; dropping the keepalive reddened that exact check |
| PW-101 | DONE | P0 | `map_range` rollback could clear an identical mapping that existed before the failed call | D-185 preflights each transaction as wholly absent or wholly identical and refuses a mixed range before mutation | 2026-08-27 QEMU `a refused remap preserves every older entry` retained the identical prefix and conflicting leaf; ignoring the preflight reddened that exact check |
| PW-102 | DONE | P0 | `mem::with_active_space` gave every safe kernel module unrestricted `&mut AddressSpace` over `Reach::Everywhere`, including reclaim and arbitrary unmap of the live tables | D-189 keeps the borrow private to `mem`; production exposes only guarded-stack and device mapping, while self-test removal consumes an unforgeable token for the exact mapping it created | 2026-08-27 QEMU retained all 194 mapping/device/stack/unmap checks; exact source proof `the running space has no broad safe borrow` passed, and exporting the private closure reddened that host test |
| PW-103 | DONE | P1 | Every failure after `AddressSpace::for_user` in `user::spawn` could abandon tables, mapped image/stack pages, shared mappings and later objects | D-191 keeps the never-run process in one rollback transaction through reserved-slot commit and preserves reusable task-stack identity on abort | 2026-08-27 QEMU `every refused spawn returns every resource` injected all fourteen space/image/stack/shared/object/handle/thread/arrangement edges twice and restored exact frame, table, heap, arena, handle, channel, ring, slot and process-record baselines; skipping table reclaim or arrangement's scheduler release reddened only that check, while restoring stack erasure reproduced the exact pre-verdict fault |
| PW-104 | DONE | P1 | A space recorded only 32 owned table frames and intentionally lost later ones; `Option` let the process collector report a refused table cleanup as zero returned | D-190 grows table ownership fallibly before taking each frame, returns every recorded frame, and makes refusal a typed result which process collection logs and counts | 2026-08-27 QEMU `a large space returns every table frame` returned all 37 frames from 34 sparse mappings; the controlled bound-space refusal changed no frame tally, forty launch/exit cycles added no refusal, and restoring the 32-entry ceiling reddened only the large-space check |
| PW-105 | DONE | P1 | Leaking a safe `SpinLockGuard` with `mem::forget` permanently strands interrupt/lock state; Rust cannot assume `Drop` runs | D-194 bounds deliberate guard loss as a liveness violation and retains every D-66 transition refusal; shipped code gets no unsound recovery door | 2026-08-27 release QEMU used real safe `mem::forget` behind a controlled recovery owner, distinguished ordinary contention from forgotten ownership, kept the lock unavailable, and restored its lock/accounting/interrupt state; suppressing the state transition reddened only `forgotten lock ownership is distinct from contention` with 198/199 |
| PW-106 | DONE | P1 | Queue teardown consumed a pending operation and only `debug_assert!`ed if its reserved completion could not be posted | D-192 retains a settling operation until its terminal record posts and retains the complete zombie until its ring is quiet | 2026-08-27 release QEMU injected refusal through cancellation, expiry and a real faulting process; pending ownership and reservations remained live, cancellation reported pressure, the first collection retained the zombie and retry posted exactly once before reclaim; deleting restoration reddened only `a refused settlement retains every owner` with 196/197 |
| PW-107 | DONE | P1 | The guarded boot-stack allocation could fail and the kernel still launched ring 3 on the unguarded bootloader stack | D-193 records the actual continuation path and refuses process construction before allocation unless the boot stack is guarded | 2026-08-27 release QEMU injected the exact missing-guard state, returned `UnguardedBootStack`, recorded no syscall crossing and preserved the complete resource ledger; the state restored for later user checks, while bypassing the gate launched a real program and reddened only `an unguarded boot stack closes the user boundary` with 197/198 |
| PW-108 | DONE | P2 | The copied boot memory map silently capped evidence at 128 regions | D-224 moves the allocation-free map into `libs/memory-map` and refuses an empty or oversized handoff whole instead of returning a prefix | 2026-08-29 host maps retained all policy-relevant tail entries at 127/128, rejected 129 with the exact count and proved allocator/direct-map/exclusion answers; restoring silent overflow reddened only `the_region_limit_is_all_or_nothing`, retained as mutation 236 |
| PW-109 | DONE | P2 | The ELF entry was rejected safely at the final crossing but only after allocation, while `PLACED` was global last-load state | D-191 validates entry-in-executable-load during preflight and returns immutable placements in `LoadedImage` | 2026-08-27 QEMU refused an entry outside executable program memory with no frame-count change, reclaimed every negative-test scratch space and retained two shifted successful loads' independent placement records; bypassing entry preflight reddened only the loader check |

## PCI, NVMe, FAT and filesystem resilience

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-200 | DONE | P0 | After a doorbell, timeout/error could drop Identify buffers, queue frames or controller state while the device might still DMA | D-188 requires a verified Controller Reset before release and otherwise makes `DmaSegment` permanently inaccessible and unrecyclable | 2026-08-27 delayed fake completion remained retained after an unverified stop; QEMU `uncertain dma memory is never recycled` withheld the frame and `an nvme timeout quiesces before dma memory returns` drove a real read through reset and observed `RDY = 0`; D-232 routes the forced timeout through the production terminal arm, whose direct-return break failed only that exact check at 214/215 |
| PW-201 | VERIFY | P0 | D-187 replaced the split live probe with one decode-off, wide-aware, fully restored transaction; the current implementation still needs its required VMware observation | Run PW-003 against D-187's transaction and record the discovered NVMe BAR | 2026-08-27 host fake configuration injected every probe cycle and proved exact order/rollback; QEMU booted NVMe at the correct 64-bit BAR; split, no-restore, no-decode-off, malformed-mask and malformed-slot mutations each reddened their exact test; current VMware must confirm the BAR |
| PW-202 | DONE | P0 | PCI claims keyed the full offset-bearing address, committed before fallible work, and could strand device/port ownership despite the unchanged-on-error contract | D-187 canonicalizes BDF identity and makes registry preparation plus all I/O windows all-or-none | 2026-08-27 two offsets could not claim one QEMU bridge twice; fake preparation failure at every edge left `ClaimRegistry` empty; every possible multi-span conflict stranded nothing; canonicalization, early-commit and partial-port mutations reddened their exact checks |
| PW-203 | DONE | P1 | `read_blocks` permits zero, exceeds the 16-bit NLB field, does not bound namespace end/overflow, and does not prove DMA length fits `Segment` | D-195 makes the private submission path accept only `libs/nvme::ReadTransfer`, validated against command width, namespace and the exact `DmaSegment` length | 2026-08-27 host tests exhausted all 65,536 encodable counts and the empty, end, wrap, multiplication and short-buffer edges; release QEMU refused five malformed raw requests before command/interception counts changed, while the integration bypass reddened only `invalid nvme reads stop before submission` with 199/200 and the interceptor kept real MMIO untouched |
| PW-204 | DONE | P1 | Namespace Identify trusted LBADS and ignored malformed/unsupported format metadata; a hostile device could shift by 32 or choose an unusable block format | D-196 makes `libs/nvme::Namespace` prove the non-empty extent, negotiated table, selected format, 512-byte size and absence of metadata/protection before I/O queue creation | 2026-08-27 host properties exhausted the NLBAF, FLBAS, LBADS, LBAF control and DPS byte domains plus every metadata size; nine defects reddened their exact host check, while the kernel bypass reddened only `invalid nvme namespace stops before io queues` with 200/201 and the interceptor prevented allocation and MMIO |
| PW-205 | DONE | P1 | A ring-3 Read polled the controller while serving a trap and could hold interrupts/controller locks for a large bounded interval | D-200 admits validated Reads to one fixed eight-slot worker queue; the preemptible worker owns runtime controller access, and queued/running states define cancellation and teardown | 2026-08-27 release QEMU held a real NVMe command after its doorbell while `RingEnter` returned, timer and injected keyboard IRQs progressed, and a retired ring stayed pinned until one exact completion; eight paused requests filled the queue, the ninth returned `STATUS_NO_ROOM`, and all admitted work settled; queue-depth, running-cancellation and trap-wait breaks reddened only their named checks |
| PW-206 | DONE | P1 | Bus-master enable failure logged but still started NVMe, and a partial command write was not rolled back | D-197 makes command enablement a verified, exactly restored transaction and makes its success the only constructor of an NVMe-mappable BAR0 extent | 2026-08-27 host fakes covered already-enabled/success/refusal, every access-loss edge and unverifiable rollback; omitting bus master, readback or rollback reddened their exact checks, while restoring log-and-continue reddened only `a failed bus master enable stops before nvme mapping` with 201/202 |
| PW-207 | DONE | P1 | Early PCI failures returned before tmpfs/devfs survey despite the claim that those backends exist whatever the bus held | D-198 makes PCI discovery, the hardware-free namespace survey and interrupt bring-up sibling boot steps; the degraded image forces the production CF8/CFC claim to refuse | 2026-08-27 degraded QEMU logged the PCI refusal, enumerated tmpfs, resolved both devfs devices, refused x2APIC and settled; restoring the PCI-conditioned survey removed those lines and the harness failed exactly on missing `tmpfs:` |
| PW-208 | DONE | P1 | Enumeration attempted only the first NVMe candidate, so one broken controller masked a later good one | D-199 gives discovery a pre-commit lease and advances only after verified controller, mapping, frame, command, port and function retirement | 2026-08-27 release QEMU retired an earlier namespace-less real controller to the exact free-frame, mapping-count, device-window and PCI-claim baseline, then mounted and read the corpus-backed sibling; leaked mapping ownership/cursor and stopped-selection mutations reddened `a later nvme controller survives an earlier failure`, while real I/O-window reacquisition caught omitted dynamic-port release |
| PW-209 | DONE | P2 | Readiness and command timeouts were iteration guesses even after a monotonic clock existed | D-200 gives `libs/nvme::PollLimit` a monotonic deadline when time exists and an explicit attempt bound during early bring-up | Host tests proved iteration-independent deadlines, exact clockless exhaustion and saturation; each defect was broken and reddened only its exact test, while release QEMU completed the real delayed-command route within the five-second policy |
| PW-210 | DONE | P1 | The FAT oracle used two readers but one image writer, so a shared writer misconception could pass both | D-207 pins a dosfstools/mtools volume as exact sparse sectors, verifies its package and image provenance, and reads it independently of the unchanged boot corpus | 2026-08-28 raw inspection proved a multi-cluster root, deleted/live and long-name slots and a fragmented 40-cluster file; both readers agreed on 44 root entries and the fragmented, long-name and nested payloads; duplicate-sector and first-cluster-only breaks reddened their exact tests and persist as two mutations |

## CPU, time, interrupts and scheduling

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-300 | DONE | P0 | CR0.WP was neither enabled nor read back, so supervisor writes to read-only kernel pages depended on bootloader state | D-186 puts WP in the boot protection contract and gives its controlled fault an exact private recovery window | 2026-08-27 QEMU `supervisor writes obey read-only pages` drove WP through the kernel setter, then recovered only the expected supervisor write fault on `.rodata`; omitting the bit reddened that exact check |
| PW-301 | VERIFY | P1 | D-201 makes active NX and calibrated progress mandatory before ring 3, but its accepted-contract proof still needs the current VMware observation | Run PW-003 and record NX, calibration and successful user launch together | 2026-08-28 release QEMU injected missing NX before allocation with no syscall or resource change; bypassing it reddened only `no-execute protection closes the user boundary`; degraded `qemu64` retained `wp nx` without SMEP/SMAP/UMIP and refused userspace for its missing clock; current VMware must complete the proof |
| PW-302 | DONE | P1 | Failed clock calibration deliberately still launched a user process with no preemption and admitted Timer promises with no progress source | D-201 gates user launch and Timer admission on the live calibration predicate | 2026-08-28 release QEMU injected calibration loss, refused launch before allocation or syscall crossing, and posted one immediate `STATUS_UNAVAILABLE` Timer completion with no pending state; bypassing the user and Timer gates separately reddened only `an uncalibrated clock admits no user or timer`, and degraded QEMU remained responsive with ring 3 closed |
| PW-303 | DONE | P1 | VMware already produced one unexplained calibration refusal; suspend, rate change, instability detection and revocation were absent | D-225 permits a complete invariant CPUID 0x15 ratio to replace refused PIT TSC calibration, retains the PIT as an independent APIC-delivery-sampled validator, and defines conservative post-boot revocation; leaf 0x16 is diagnostic and suspend remains unsupported | 2026-08-29 host tests separated exact 0x15 evidence, nominal 0x16 MHz, stopped/early disagreement, stale APIC delivery and late inconclusive windows; release/debug QEMU injected two conclusive failures through the production policy, froze monotonic time, preserved private scheduler progress, survived the time-page mapping gap and terminalized an admitted Timer; all six breaks reddened only their named checks and persist as mutations 237–242 |
| PW-304 | DONE | P1 | ACPI retained only the first I/O APIC, so a valid GSI owned by a later controller could not be routed | D-205 inventories up to sixteen controllers, probes and masks every hardware-sized range under releasable ownership, rejects ambiguity and retains only the unique owner | 2026-08-28 host tables retained two controllers and refused bounded overflow; pure range tests rejected empty, overflowing and overlapping ownership and selected a later owner across a gap; all five exact mutations reddened their named tests; release QEMU returned the probe mapping, remapped the selected owner, routed GSI 1 and passed all 211 checks |
| PW-305 | VERIFY | P2 | D-228 now observes every enabled component independently; the standard form is proven locally, while compacted restore still needs its current hardware observation | Run PW-003 and require the same x87/XMM/YMM-high switch check under the compacted `XSAVEC` banner | 2026-08-29 release QEMU passed all 213 checks through standard `XSAVE`; clearing x87, SSE and AVX each reddened only `every enabled state component survives a switch` at 212/213 and persists as mutations 259–261; a first push-only x87 witness falsely stayed green until each write reset the x87 stack; current VMware must pass that named check through `XSAVEC` |
| PW-306 | DONE | P1 | The idle-start latch was set before spawn succeeded, so a transient full table prevented idle from ever being retried | D-202 reserves a three-state idle attempt, rolls back every spawn refusal, and classifies the slot before making it runnable | 2026-08-28 release QEMU filled every spawnable slot, refused idle creation without consuming its retry, freed the fillers and created exactly one idle thread despite two retries; burning the reservation reddened only `a failed idle start can be retried exactly once` with 208/209 |
| PW-307 | DONE | P1 | A zombie progressed only at idle or a later spawn and was bounded rather than deterministically reaped | D-203 makes retirement wake the mandatory service worker, which alternates bounded reclamation with file work and gates ring-3 launch on its readiness | 2026-08-28 release QEMU kept normal work runnable while the worker reclaimed a retired program without another spawn, direct collection or idle halt and restored the exact resource ledger; missing-worker and missing-wake mutations each reddened only that check with 209/210 |

## Hostile boundary, concurrency and accounting

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-400 | DONE | P1 | Production boundary code retained four panic assumptions, and FAT accepted geometry whose later arithmetic could panic in debug or wrap in release | D-206 removes the unused dynamic-port access API, guards target boundaries with compiler/source rules, validates FAT geometry at construction and walks offsets by quotient/remainder | 2026-08-28 each of three FAT arithmetic refusals and two source rules failed for its named reason; five persistent mutations retain them; release/debug QEMU mounted and read the valid corpus and passed all 211 checks |
| PW-401 | DONE | P1 | Shared-ring prose invoked READ_ONCE, but page copies were ordinary memory operations; the UP trap path serialized kernel mutation without a compiler/SMP contract | D-221 makes each index the fixed atomic acquire/release edge, payload copies volatile at the `Shared` boundary and concurrent kernel completers one logical producer | 2026-08-29 the source-bound four-event model excluded published-index/old-payload only with both edges; independent Relaxed release/acquire, ordinary-store, wrong-client-index and split-producer breaks each reddened its exact host witness and persist as mutations; release/debug QEMU exercised the real client at 212/212 |
| PW-402 | DONE | P1 | Table identity was a wrapping `u32`; eventual reuse could collide with still-live ownership, independently of the documented 12-bit handle generation tradeoff | D-204 makes identity a non-repeating `u64`, commits its checked successor with quota under the arena lock and returns typed exhaustion at the terminal sentinel | 2026-08-28 host arithmetic refused the maximum successor; release QEMU held the last table and handle live through two fail-atomic refusals and exact teardown, while wrap and final-nonadvance mutations each reddened only `table identities never wrap or repeat` with 210/211 |
| PW-403 | DONE | P2 | The handle mixer exposed `NotYours` versus `Stale`, one foreign-occupancy bit per exact slot-generation probe even though no authority crossed | D-222 collapses empty, reused and foreign slots to `Stale` while retaining the ownership checks on resolve and close; the API result now carries zero occupancy bits | 2026-08-29 the real-arena experiment observed identical live-foreign and closed/absent resolve and close results while the owner retained its entry; removing either ownership check reddened only `a handle reveals no foreign occupancy` and both breaks persist as mutations; the offline gate passed release/debug QEMU at 212/212 plus shipped and degraded boots |
| PW-404 | DONE | P2 | The count-only TLA+ model and sequentially consistent Shuttle schedules did not compose shared-memory visibility with terminal ownership | D-229 adds a finite identity-bearing weak-memory model and makes the supported model task require four exact bad configurations to fail | 2026-08-29 OpenJDK 21.0.5 / TLC 2026.08.21.155922 completed the original model at 1,040 distinct states/depth 49 and the new publish/admit/normal/cancel/retire/reap model at 1,296/depth 31; relaxed store/load, lost reservation and duplicate terminal each violated only its named invariant; removing the real reservation, settling guard and retirement transition reddened their source/model bindings and persists as mutations 262–264 beside D-221's ordering mutations |
| PW-405 | DONE | P2 | The constructible syscall wake/arm race and guard `!Send` compile-time property lacked durable automated witnesses | D-223 drives a real child `RingEnter` only after a kernel waiter registers on its ring, and cross-compiles `Send` assertions against both actual `wost` guard types | 2026-08-29 release QEMU observed both waits, one exact RingEnter arm, Echo ownership and exact cleanup; restoring the trap exemption reddened only `a syscall wake arms the timer for what it woke` at 211/212, while removing either CPU-local marker made the named host compile-fail test red; mutations 234 and 235 retain the type breaks |

## Verification, release engineering and repository truth

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-500 | DONE | P2 | `image::build` still accepted `&[&str]`, allowing shipped/self-test image confusion | D-211 makes `Kernel::{Shipped, SelfTest}` the complete normal-image selector and gives the degraded witness its own fixed constructor | 2026-08-28 release shipped and self-test images reopened their FAT payload, matched the verified ELF byte for byte and identified as requested; mapping self-test to no features and reading the program in place of the kernel reddened their exact host checks and persist as mutations |
| PW-501 | DONE | P2 | CI composition and prose counts were partly inferred from source text, and README already claimed three Cargo graphs where four ran | D-212 makes one typed registry own every Cargo graph and ordered local-gate stage; execution and help iterate it, while README carries its exact checked Markdown rendering | 2026-08-28 the unchanged four-graph, nine-stage gate rendered identically in help and README; removing the `refuse-pci` graph and adding a duplicate shipped boot each reddened `the_ci_registry_drives_help_and_documentation` and persist as mutations |
| PW-502 | DONE | P1 | The mutation runner edited tracked source in place; interruption could leave a live worktree modified | D-208 runs from a locked tracked-file copy with a private build directory and alternating checksummed progress journals | 2026-08-28 real child processes killed at build and boot left an active journal and reusable lock; each resume restored the sandbox exactly while the source and index stayed byte-exact; isolation, private Cargo target, live-lock, restoration and newest-journal breaks reddened their exact host tests and persist as five mutations |
| PW-503 | DONE | P2 | Host/library break-tests were one-off and could silently stop reaching production code | D-210 retains one high-consequence production-source mutation in every pure crate and derives the coverage floor from `HOST_PACKAGES` | 2026-08-28 all sixteen newly represented crates compiled their deliberate defect and reddened the measured exact host test; the ring's plausible test stayed green and its interleaving oracle was pinned instead, a zero-test timebase name was refused and corrected, and two more mutations prove the coverage floor and exact-test execution can fail |
| PW-504 | DONE | P2 | A mutation can look caught when its named check was already red or another check failed first | D-209 captures one pristine green baseline per remaining witness class and admits only an exact named check or panic marker afterward | 2026-08-28 pre-red, wrong-check, wrong-panic, discarded-continuation and unnamed-failure breaks each reddened only their exact host check; three terminal mutations were pinned to causes measured on release QEMU, and all six new witnesses persist in the registry |
| PW-505 | DONE | P2 | The public `libs/`/`wost` API sweep was a text search and `xtask` still duplicated section-table ELF header offsets | D-213 adds a stable-rustc semantic reachability audit and makes `libs/elf64::SectionTable` own those four file-header fields | 2026-08-28 all 1,466 item declarations and enum variants had compiler-resolved callers after 29 dead APIs were removed; re-export, doc-only and same-name fixture defects each reddened the exact host test, the ELF-offset defect reddened its parser test, and restoring a dead API made the full audit name only that method; all five persist as mutations |
| PW-506 | DONE | P2 | `wost` has 368 unsafe lines against a 380-line budget and SAFETY prose truth can only be read | D-214 pins every unsafe-bearing file's full reviewed revision, makes the counter syntactic and closes two safe-wrapper lifetimes | 2026-08-28 all 169 blocks and 38 declarations in 18 files were read; hardware-pinned stacks refuse inactive roots and every overlapping unmap, the syscall probe cannot return open, and five exact mutations reddened their host or 212-check machine witness |
| PW-507 | DONE | P2 | Documentation checking did not cover Markdown links/anchors, CLI claims, current numeric claims, or docs 008-012 from D-146 | D-215 checks local Markdown targets and fragments, parses every formatted xtask command with the real CLI, derives current counts, and dates the 008-012 numeric record | 2026-08-28 missing path, missing heading, unknown command and stale current-count mutations each reddened only their named host proof; the Phase 4 close was corrected from D-76's 75-check checkpoint to D-77's 79 |
| PW-508 | DONE | P2 | A release lacks one reproducible evidence manifest tying source, tools and booted bytes together | D-216 makes one clean qualification bind the gate, two fresh locked builds and the exact booted artifact into an offline-verifiable manifest | 2026-08-28 commit `b88e752` passed the complete gate; both 3,462,408-byte kernel ELFs hashed `082e09c7…8945f`, both 64 MiB images hashed `4daa81dd…221dc`, the bundled path-A image responded and settled without changing, and a separate offline invocation verified every field; all nine new refusals reddened their exact host checks and persist as mutations |
| PW-509 | DONE | P2 | QEMU machine aliases, firmware, VMware, Java and Rust versions can drift underneath evidence | D-218 pins the emulated machine ABI and makes evidence schema v2 record fixed-shape identities for every local and exceptional qualification tool | 2026-08-29 QEMU used `pc-q35-9.2`; TLC ran through the resolved Microsoft OpenJDK 21.0.5 and VMware generation named `vmrun` 1.17.0 build 24583834 without launching it; alias, version parsing, optional-schema and missing-dependency breaks reddened their four exact host witnesses and persist as mutations |
| PW-510 | DONE | P2 | D-16 deferred advisory/license/SBOM policy, yet the project now vendors boot and test artifacts and ships dependencies | D-220 defines one lock-bound inventory, a deterministic SPDX SBOM, distribution paperwork, a hydrated offline-build contract and an exceptional fresh-advisory review | 2026-08-29 all 67 registry packages and three vendored artifacts matched exact committed digests, the kernel closure named four runtime inputs, SPDX regenerated byte-identically and the complete 11-stage gate passed with Cargo offline; seven dependency, scope, license, digest, notice, SBOM and ambiguous-version breaks each reddened only its exact host witness and persist as mutations |
| PW-511 | DONE | P2 | No stable performance/resource baseline said when a correctness-preserving change regressed the foundation | D-230 adds a bounded three-release/three-debug fixed-QEMU baseline over exact host artifacts, steady resources and real ring-3/NVMe paths | 2026-08-29 the tightened task passed six 215-check boots: release 5.11 to 5.38 ms / 24.6 to 27.0 microseconds trap / 26.9 to 33.5 MB/s storage and debug 26.8 to 29.5 ms / 105.9 to 201.7 microseconds / 7.5 to 8.7 MB/s; image, unsafe and retained-state bands held, the exploratory debug stack overflow was removed, and six exact breaks persist as mutations 266 through 271 |
| PW-512 | DONE | P2 | QMP probe/rebind had a local port race; shared firmware/image creation and unescaped QEMU drive paths also collided under concurrent runners | D-227 reserves both control listeners, gives every VM private firmware and boot-image state, serializes shared corpus construction, snapshots shared media and encodes every drive path | 2026-08-29 release QEMU passed 213/213 through the real comma/space paths, shipped HMP/QMP liveness advanced IRQ 24→30, and two simultaneous shipped boot checks both completed from distinct images and ports; eight exact host breaks persist as mutations 251–258 |
| PW-513 | DONE | P2 | There was no supported Windows debugger task, slowing exact boundary work | D-231 pins native WinLibs GDB 17.2 and gives one private, dynamically addressed paused QEMU run both interactive and bounded-proof modes | 2026-08-29 `cargo xtask debug --check` hit `wizard_kernel::kmain`, resolved `wost/src/boot/mod.rs:526`, printed frame zero and exited in 3.3 seconds; interactive GDB reached the same prompt and `quit` reaped QEMU, no run left a process, and five exact breaks persist as mutations 272 through 276 |
| PW-514 | DONE | P2 | One real `redundant_clone`, stale historical prose and a stale i8042 port-claim comment were already named | D-219 removes the lint, resolves the bootloader contradiction and makes current summaries name later decisions without rewriting dated evidence | 2026-08-29 the targeted nursery lint, all-graph Clippy, rustdoc and 193-test host suite passed; restoring the retired own-bootloader roadmap failed only `current_facing_foundation_prose_names_the_later_resolutions`, and mutation 219 retains it |
| PW-515 | DONE | P1 | The bootstrap-authority check observed a short-lived program after making it runnable and paired a global retirement count with a reusable slot's stale exit record | D-217 captures immutable birth evidence before runnability and keys every exit observation to the same non-repeating table identity | 2026-08-29 debug QEMU passed 212/212 after a clean-tree comparison reproduced the timing exposure; returning an unrelated parent channel reddened only `a spawned program holds exactly one handle` with 211/212 and persists as a mutation |
| PW-516 | DONE | P1 | D-84 scoped the property tier to `libs/ring-proto` only, so twenty-four crates carried nothing but inputs a person chose, and D-241's three contradicting contracts were reachable only by checking a stated claim against code that stayed total on every input | D-242 admits a check when it cites an invariant it did not read off the code, requires a generated-input check to state its measured reach, and carries the tier into `memory-map`, `path-walk`, `stack-layout`, `port-space` and `frame-space` | 2026-08-29 all five passed with reach stated in each — 4,096 of 4,096 maps built and 2,875 yielding a run, 2,863 paths accepted against 1,233 refused, 3,716 multi-slot windows, 3,510 grants against 586 refusals and 20,214 allocations against 230 exhaustions; letting a bare dot survive as a component and making only the left range empty each reddened their exact property and persist as mutations 295 and 296 |
| PW-517 | DONE | P1 | A finite domain that is sampled is a weaker claim than one that is walked, and nothing in the tree walked one; `libs/slab-layout`'s size classes are small enough to enumerate completely and were checked on fourteen chosen cases instead | D-242 admits bounded exhaustion as a tier of its own, needing no dependency at all, with each test naming its bound so that a bound which shrinks is a visible edit rather than a silent one | 2026-08-29 `libs/slab-layout/tests/exhaustive.rs` walked all 53,261 size and alignment pairs with both the satisfied and the too-big arm reached, every class index round-tripped inside its documented range, and every slot of every class was class-aligned and tiled its frame exactly |

## Compiler gate and repository handoff

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| WZ-001 | WIZZY | GATE | The inspected compiler revision was local-only | Publish/reachably pin one `wizards-ink` revision, toolchain and every dependency | A clean machine can fetch exactly the recorded inputs; no sibling working tree is implied |
| WZ-002 | WIZZY | GATE | Wizzy has no non-interactive standalone program artifact | Add one documented command from standalone source to a versioned loadable artifact | Artifact identifies/rejects versions and runs without compiler, checked AST, compiler maps or host services |
| WZ-003 | WIZZY | GATE | The VM/runtime retains compiler structures and uses `std` host facilities | Separate the execution runtime behind a host-independent platform seam and build it with `no_std + alloc` | Target runtime dependency audit contains no compiler/AST/host I/O; host and minimal target adapter run the same fixture |
| WZ-004 | WIZZY | GATE | Reproducible compiler output is not established | Lock inputs and make two clean builds byte-identical | Gate record includes both commands, environments and matching artifact hashes |
| PW-600 | DONE | GATE | The shared handoff was described but had no single machine-readable version record | D-226 defines an exact route-neutral envelope and allocation-free parser without inventing the compiler-owned artifact/runtime/effect versions | 2026-08-29 the committed opaque fixture and digest verified without a sibling checkout; unknown format/fields, malformed encodings, every target/ABI mismatch, one-of-two identity matches and platform-version drift were refused, and all eight breaks persist as mutations 243–250 |
| PW-601 | TRIGGER | WHEN-TRIGGERED | Consumer code before a real artifact would freeze guesses | Add only the loader/runner scaffold forced by WZ-002/WZ-003 after their command and format draft are reachable | Trigger: a pinned draft artifact plus runtime seam; proof consumes committed/vendored bytes and compares regenerated digests separately |
| PW-602 | TRIGGER | WHEN-TRIGGERED | Opening from a developer checkout would make the gate non-reproducible | Write the D-183 gate-opening ADR only after WZ-001 through WZ-004 pass together | Trigger: all four rows at one revision; ADR records pins, commands, hashes, seam and keeps ordinary OS CI sibling-independent |

## Trigger-only kernel/platform work

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-700 | TRIGGER | WHEN-TRIGGERED | Frame quotas and per-message allocation accounting have no current user-controlled allocation route | Add accounting at the admission owner selected by the actual runtime/channel caller | Trigger: first user spawn/map/channel-send route; over-quota refusal leaks nothing and cannot starve another process |
| PW-701 | TRIGGER | WHEN-TRIGGERED | Dynamic stacks and a first-class `MemoryObject` need measured runtime allocation/zero-copy behavior | Design them against the chosen Wizzy runtime rather than today's probe | Trigger: post-gate route measurement; repeated growth/shrink/exit is W^X, guarded and resource-steady |
| PW-702 | TRIGGER | WHEN-TRIGGERED | Usercopy fault fixup has no unmap race while a process has one thread | Stress/fix the race when another thread can revoke the copied range | Trigger: multi-threaded process or asynchronous unmap; no stale access, livelock or leaked recovery state |
| PW-703 | TRIGGER | WHEN-TRIGGERED | Stack-token middle-page validation matters only when stacks grow or shrink | Make stack evidence linear or validate its complete dynamic extent | Trigger: dynamic stack mutation; removing any page invalidates the token before CPU installation |
| PW-704 | TRIGGER | WHEN-TRIGGERED | Page cache and deferred storage throughput policy need a workload, not intuition | Measure the first real Wizzy file workload and decide cache/writeback ownership | Trigger: pinned Wizzy workload after route integration; numbers and memory pressure justify the design |
| PW-705 | TRIGGER | WHEN-TRIGGERED | Limine/base-revision upgrades, runtime logging controls and extra ABI integer/buffer types have no current forcing event | Re-evaluate only on stable support, configuration source, or source-level Wizzy caller respectively | Trigger is named in the change; no dependency/API upgrade lands solely because it exists |

<!-- PRE-WIZZY-REGISTER:END -->

## Explicitly after the compiler gate

These are designed in `015-wizzy-userland.md`, but they are not pre-Wizzy
kernel work: ring-3 channel create/send/receive, handle close, the bootstrap
message, terminal and directory delegation, enumeration, VM map/unmap and
runtime allocation, the Wizzy platform adapter and capability values, process
objects, spawn-from-image, exit status, `init`, TTY, shell, pipelines, and
tmpfs mutation. Their order begins with a measured real artifact; none may be
pulled forward by relabeling it "foundation".

The current fixed process/stack capacity remains until that measured route
shows what density is required. Loader relocations, TLS and image extensions
also wait for the actual linked artifact. The user-runtime `unsafe` boundary
is decided with that runtime, not for an imagined one.

## Later roadmap and deliberately absent work

| Phase/standing | Work that does not move into this register |
|---|---|
| Phase 8 | randomness and networking; kernel pieces remain Rust, first-party target services are Wizzy |
| Phase 9 | POSIX/mlibc completion, signals, compat-fork, dynamic linking and compatibility ports/runtimes |
| Phase 10 | GPU, compositor, toolkit and graphical terminal |
| Phase 11 | NetSurf document-web integration |
| Phase 12 | SMP and its stress work, GS/swapgs, KASLR, CET/IBT/XFD callers, signed/measured boot, USB, sound and real hardware qualification |
| Phase 13 | writable persistent WizFS, crash consistency, repair and administration tools |
| Deliberately absent | xAPIC fallback for out-of-scope old CPUs, kernel Wizzy runtime/compiler, Rust fallback userland, speculative POSIX/native ABI growth, full functional verification, and FRED implementation without a hardware/toolchain caller |

The missing dated Phase 3 VMware result cannot be reconstructed. Phase 0-5
VMware boots that used non-self-test images remain liveness evidence only.
Neither is implementation debt, and neither may be upgraded retroactively.

## Audit cautions and prose cleanup resolution

The static audit deliberately did not promote these observations into P0
claims: NVMe polling is bounded (the defect is DMA lifetime after its timeout);
the final ring-3 crossing already refuses a non-present, non-user or
non-executable ELF entry (PW-109 is earlier/cleaner refusal); and the two
`map_shared` callers happened to keep their backing alive, so PW-100 was an
unsafe *safe API* rather than a demonstrated exploit before D-185 closed it.
D-201 records NX as mandatory for ring 3 and SMEP/SMAP/UMIP as preferred
optional hardening. The current VMware observation still belongs to PW-301
and PW-003; D-225 closes PW-303's clocksource qualification and post-boot
revocation without upgrading that pending VMware observation.

PW-514 carried these exact stale-current-facing corrections; D-219 closes all
of them without upgrading historical evidence:

- `011-userspace.md` now gives leaf-closure, `MappedStack` and broad active-space
  access their D-121, D-125/D-214 and D-189 dispositions;
- `011` names D-154's per-CPU seam, separates D-164's timed wait from the
  uncalled multi-queue wait, and explicitly says first-class `MemoryObject` and
  dynamic stacks do not exist;
- `009-multitasking.md` marks its close as historical and names D-164, D-89 and
  D-92 as the later payment of timed waits and both stack guards;
- `001-boot.md` names the shipped initrd module;
- `012-research-notes-4.md` records D-162's anchored set-2 backslash code; and
- `000-vision.md` and D-03 now follow D-219: Limine remains the boot boundary,
  and an own loader needs a concrete unmet handoff rather than a roadmap slot.

## What is left

D-185 through D-238 closed every code-bearing row in this register, and the
closing qualification ran on `ff6ddd3`: the gate, the complete mutation sweep,
the ring models and the offline evidence manifest are all recorded above. What
remains needs a person at a machine.

1. Boot the two generated VMware machines and record both fresh transcripts
   (PW-003, PW-004). Nothing else here can proceed without them, and generating
   a machine is not booting one.
2. Close PW-201, PW-301 and PW-305 from that self-test transcript: the
   discovered NVMe BAR, active NX beside a calibrated clock and a successful
   user launch, and `every enabled state component survives a switch` reached
   through the compacted `XSAVEC` banner rather than the standard form QEMU
   offers.
3. Write the phase-close ADR, which is PW-006's deliverable and the last
   register-closing change, then run `cargo xtask ci` once more (PW-005).
4. In `wizards-ink`, and independently of all of the above, complete WZ-001
   through WZ-004. Only then execute PW-601/PW-602 and begin `015`'s post-gate
   slices.

This ordering is a risk order, not authorization to skip the register's proof
rules. Every implementation still gets its check, every new check is broken
once, and every machine check gets a persistent mutation.
