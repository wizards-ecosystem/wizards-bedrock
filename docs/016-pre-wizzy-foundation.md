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
are **not closed**. The current image has 206 QEMU machine checks. The current
mutation registry has 136 entries, but the last recorded complete sweep covered
64; the seventy-two later mutations were broken individually. The current
206-check image has not completed its human VMware round. The compiler gate in
`015-wizzy-userland.md` is also **CLOSED**.

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
| PW-001 | REQUIRED | P0 | The register contains 136 mutations, but only the former 64-entry sweep has a complete recorded result | Run `cargo xtask mutate` from a clean tree on the closing commit; preserve per-mutation causality | All 136 named mutations are caught by the check each names; record commit, result and duration; tree is byte-for-byte restored |
| PW-002 | REQUIRED | P0 | The ring model is a phase-close gate outside CI | Re-run `cargo xtask model-check` on the closing commit | Record TLC/Java versions, generated and distinct states, depth, zero errors and exact commit |
| PW-003 | REQUIRED | P0 | The dated VMware result predates D-181/D-182 and covered 182/187, not the current 206-check image | Boot the current self-test image in VMware and perform every human keyboard round | A dated `passed=206 failed=0` transcript, including WP, SMEP/SMAP/UMIP, compacted XSAVE, x2APIC, storage and input evidence |
| PW-004 | REQUIRED | P1 | A self-test image is not the image users boot | Boot the release/shipped image in VMware after PW-003 | It reaches its prompt, storage/VFS survey and user boundary without panic; record image digest and VM/tool version |
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
| PW-108 | VERIFY | P2 | The copied boot memory map silently caps evidence at 128 regions | Decide refusal, checked coalescing, or dynamic bounded storage rather than silently omitting firmware regions | Synthetic maps above and around the limit prove allocator, direct-map and exclusion decisions cover the same complete input |
| PW-109 | DONE | P2 | The ELF entry was rejected safely at the final crossing but only after allocation, while `PLACED` was global last-load state | D-191 validates entry-in-executable-load during preflight and returns immutable placements in `LoadedImage` | 2026-08-27 QEMU refused an entry outside executable program memory with no frame-count change, reclaimed every negative-test scratch space and retained two shifted successful loads' independent placement records; bypassing entry preflight reddened only the loader check |

## PCI, NVMe, FAT and filesystem resilience

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-200 | DONE | P0 | After a doorbell, timeout/error could drop Identify buffers, queue frames or controller state while the device might still DMA | D-188 requires a verified Controller Reset before release and otherwise makes `DmaSegment` permanently inaccessible and unrecyclable | 2026-08-27 delayed fake completion remained retained after an unverified stop; QEMU `uncertain dma memory is never recycled` withheld the frame and `an nvme timeout quiesces before dma memory returns` drove a real read through reset and observed `RDY = 0`; direct-return, false-quiescence and recycle mutations each reddened their exact check |
| PW-201 | VERIFY | P0 | D-187 replaced the split live probe with one decode-off, wide-aware, fully restored transaction; the current implementation still needs its required VMware observation | Run PW-003 against D-187's transaction and record the discovered NVMe BAR | 2026-08-27 host fake configuration injected every probe cycle and proved exact order/rollback; QEMU booted NVMe at the correct 64-bit BAR; split, no-restore, no-decode-off, malformed-mask and malformed-slot mutations each reddened their exact test; current VMware must confirm the BAR |
| PW-202 | DONE | P0 | PCI claims keyed the full offset-bearing address, committed before fallible work, and could strand device/port ownership despite the unchanged-on-error contract | D-187 canonicalizes BDF identity and makes registry preparation plus all I/O windows all-or-none | 2026-08-27 two offsets could not claim one QEMU bridge twice; fake preparation failure at every edge left `ClaimRegistry` empty; every possible multi-span conflict stranded nothing; canonicalization, early-commit and partial-port mutations reddened their exact checks |
| PW-203 | DONE | P1 | `read_blocks` permits zero, exceeds the 16-bit NLB field, does not bound namespace end/overflow, and does not prove DMA length fits `Segment` | D-195 makes the private submission path accept only `libs/nvme::ReadTransfer`, validated against command width, namespace and the exact `DmaSegment` length | 2026-08-27 host tests exhausted all 65,536 encodable counts and the empty, end, wrap, multiplication and short-buffer edges; release QEMU refused five malformed raw requests before command/interception counts changed, while the integration bypass reddened only `invalid nvme reads stop before submission` with 199/200 and the interceptor kept real MMIO untouched |
| PW-204 | DONE | P1 | Namespace Identify trusted LBADS and ignored malformed/unsupported format metadata; a hostile device could shift by 32 or choose an unusable block format | D-196 makes `libs/nvme::Namespace` prove the non-empty extent, negotiated table, selected format, 512-byte size and absence of metadata/protection before I/O queue creation | 2026-08-27 host properties exhausted the NLBAF, FLBAS, LBADS, LBAF control and DPS byte domains plus every metadata size; nine defects reddened their exact host check, while the kernel bypass reddened only `invalid nvme namespace stops before io queues` with 200/201 and the interceptor prevented allocation and MMIO |
| PW-205 | DONE | P1 | A ring-3 Read polled the controller while serving a trap and could hold interrupts/controller locks for a large bounded interval | D-200 admits validated Reads to one fixed eight-slot worker queue; the preemptible worker owns runtime controller access, and queued/running states define cancellation and teardown | 2026-08-27 release QEMU held a real NVMe command after its doorbell while `RingEnter` returned, timer and injected keyboard IRQs progressed, and a retired ring stayed pinned until one exact completion; eight paused requests filled the queue, the ninth returned `STATUS_NO_ROOM`, and all admitted work settled; queue-depth, running-cancellation and trap-wait breaks reddened only their named checks |
| PW-206 | DONE | P1 | Bus-master enable failure logged but still started NVMe, and a partial command write was not rolled back | D-197 makes command enablement a verified, exactly restored transaction and makes its success the only constructor of an NVMe-mappable BAR0 extent | 2026-08-27 host fakes covered already-enabled/success/refusal, every access-loss edge and unverifiable rollback; omitting bus master, readback or rollback reddened their exact checks, while restoring log-and-continue reddened only `a failed bus master enable stops before nvme mapping` with 201/202 |
| PW-207 | DONE | P1 | Early PCI failures returned before tmpfs/devfs survey despite the claim that those backends exist whatever the bus held | D-198 makes PCI discovery, the hardware-free namespace survey and interrupt bring-up sibling boot steps; the degraded image forces the production CF8/CFC claim to refuse | 2026-08-27 degraded QEMU logged the PCI refusal, enumerated tmpfs, resolved both devfs devices, refused x2APIC and settled; restoring the PCI-conditioned survey removed those lines and the harness failed exactly on missing `tmpfs:` |
| PW-208 | DONE | P1 | Enumeration attempted only the first NVMe candidate, so one broken controller masked a later good one | D-199 gives discovery a pre-commit lease and advances only after verified controller, mapping, frame, command, port and function retirement | 2026-08-27 release QEMU retired an earlier namespace-less real controller to the exact free-frame, mapping-count, device-window and PCI-claim baseline, then mounted and read the corpus-backed sibling; leaked mapping ownership/cursor and stopped-selection mutations reddened `a later nvme controller survives an earlier failure`, while real I/O-window reacquisition caught omitted dynamic-port release |
| PW-209 | DONE | P2 | Readiness and command timeouts were iteration guesses even after a monotonic clock existed | D-200 gives `libs/nvme::PollLimit` a monotonic deadline when time exists and an explicit attempt bound during early bring-up | Host tests proved iteration-independent deadlines, exact clockless exhaustion and saturation; each defect was broken and reddened only its exact test, while release QEMU completed the real delayed-command route within the five-second policy |
| PW-210 | READY | P1 | The FAT oracle uses two readers but one image writer, so a shared writer misconception can pass both | Add an independently produced, provenance-pinned FAT32 image and adversarial directory/layout cases | Both readers agree on foreign and local corpora; deliberate parser disagreement is caught; license/provenance and digest are recorded |

## CPU, time, interrupts and scheduling

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-300 | DONE | P0 | CR0.WP was neither enabled nor read back, so supervisor writes to read-only kernel pages depended on bootloader state | D-186 puts WP in the boot protection contract and gives its controlled fault an exact private recovery window | 2026-08-27 QEMU `supervisor writes obey read-only pages` drove WP through the kernel setter, then recovered only the expected supervisor write fault on `.rodata`; omitting the bit reddened that exact check |
| PW-301 | DECIDE | P1 | NX is optional in code while user launch and the security baseline assume executable permissions are enforceable | State the minimum CPU contract and fail closed for ring 3 when mandatory isolation features are absent | Degraded boots distinguish supported fallback from refused user launch; QEMU/VMware evidence covers the accepted contract |
| PW-302 | DECIDE | P1 | Failed clock calibration deliberately still launches a user process with no preemption and unusable Timer/Uptime progress | Choose a real fallback clock/preemption route or gate user launch and timer admission on calibrated service | Forced calibration failure remains responsive, admits no promise it cannot complete, and cannot give an infinite user loop the CPU forever |
| PW-303 | DECIDE | P1 | VMware already produced one unexplained calibration refusal; suspend, rate change, instability detection and revocation are absent | Cost CPUID frequency leaves and at least one independent clocksource/validation path; define post-boot instability policy | Deterministic disagreement/failure injection selects or revokes a source without time going backward or pending timers hanging |
| PW-304 | VERIFY | P1 | ACPI retains only the first I/O APIC, so a valid GSI owned by a later controller cannot be routed | Represent all bounded MADT I/O APICs and select by GSI range | Host table corpus plus machine routing proof covers multiple controllers, overlap/malformed ranges and fallback |
| PW-305 | READY | P2 | XSAVE bounds are checked, but restore correctness observes XMM0 only, not every enabled state component | Add independent enabled-state coverage feasible on QEMU/VMware and explicitly bound what cannot yet be generated | Values in each claimed component survive switches; compacted and standard forms are both exercised; mutations omit one component |
| PW-306 | READY | P1 | The idle-start latch is set before spawn succeeds, so a transient full table prevents idle from ever being retried | Reserve the slot or commit the latch only after successful spawn | Fill, fail, free and retry creates exactly one idle thread; mutation burns the latch early |
| PW-307 | READY | P1 | A zombie progresses only at idle or a later spawn and is bounded rather than deterministically reaped | Give cleanup a bounded progress trigger without reclaiming on the retiring stack | A busy machine that neither idles nor spawns still reaches a steady resource baseline; race/death checks retain safe ordering |

## Hostile boundary, concurrency and accounting

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-400 | READY | P1 | Production boundary code still contains panic assumptions and indexing that deserve a hostile-input audit; four non-selftest `expect` sites are known | Remove recoverable runtime `expect`/panic/index paths or prove them unreachable by type/checked construction; add a narrow source rule where useful | Malformed ring, ELF, PCI, ACPI, FAT and device inputs never panic release/debug kernels; every new refusal is broken once |
| PW-401 | VERIFY | P1 | Shared-ring prose invokes READ_ONCE, but page copies are ordinary memory operations; the current UP trap path serializes mutation but the compiler/SMP contract is unstated | Specify the actual access and acquire/release/volatile contract, then implement it at the narrow shared-memory boundary | Litmus/stress/disassembly or an explicit model proves publication/consumption order; a weakened-order mutation is caught |
| PW-402 | READY | P1 | Table identity is a wrapping `u32`; eventual reuse can collide with still-live ownership, independently of the documented 12-bit handle generation tradeoff | Refuse identity exhaustion or widen/prove a non-colliding lifetime | Near-maximum host tests and live-table checks show no owner identity is reused while reachable |
| PW-403 | VERIFY | P2 | The handle mixer intentionally leaks a small cross-table occupancy signal even though no authority crosses | Measure the oracle and either remove it without weakening stale/owner checks or explicitly accept it | Cross-table experiment states the maximum information leaked and proves the chosen result |
| PW-404 | READY | P2 | Current TLA+ covers ring admission/accounting and Shuttle explores sequentially consistent host interleavings, not the hardware/shared-page memory contract | Extend the model or add a focused weak-memory model for publish, reserve, cancel, retire and exactly-once completion | Model/litmus mutations for reordered loads/stores, lost reservation and duplicate terminal completion all fail for the intended property |
| PW-405 | READY | P2 | The constructible syscall wake/arm race and guard `!Send` compile-time property lack durable automated witnesses | Add the smallest host/compile-fail or machine harness that reaches each actual property without copying the production type | Each harness is broken against the exact old behavior and fails for the named reason |

## Verification, release engineering and repository truth

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-500 | READY | P2 | `image::build` still accepts `&[&str]`, allowing shipped/self-test image confusion | Use a typed `Kernel::{Shipped, SelfTest}` selector through every image/VM caller | Compile-time API makes the wrong pairing unspellable; both image kinds are read back and identified |
| PW-501 | READY | P2 | CI composition and prose counts are partly inferred from source text | Make pass/boot composition single-source and data-driven without changing the self-contained local gate | Help, execution and documentation derive from one registry; mutation removes/adds a pass and the repository check fails |
| PW-502 | READY | P1 | The mutation runner edits tracked source in place; interruption can leave a live worktree modified | Run mutations in an isolated worktree/copy with lock, journal, resume and verified restoration | Kill the runner during build and boot, resume it, and prove the user's tree and index never changed |
| PW-503 | READY | P2 | Host/library break-tests are one-off and have no persistent mutation registry | Add targeted source mutations for high-value pure invariants where a stable transformation exists | Every registered host mutation still applies exactly once and its named test, not an unrelated failure, catches it |
| PW-504 | READY | P2 | A mutation can look caught when its named check was already red or another check failed first | Capture a green baseline and require the exact expected verdict delta/marker for every machine mutation | Deliberately pre-red and wrong-check cases are inconclusive/fail, never counted as caught |
| PW-505 | READY | P2 | The public `libs/`/`wost` API sweep was a text search and `xtask` still duplicates ELF offsets | Add compiler-aware reachability/dead-public-API evidence and use `libs/elf64` for host image parsing where practical | Re-export, doc-only and same-name fixtures prove the audit; no private transcription claims to be canonical |
| PW-506 | READY | P2 | `wost` has 372 unsafe lines against a 380-line budget and SAFETY prose truth can only be read | Perform a final semantic unsafe/safe-wrapper audit; reduce or deliberately revise the budget with reasons | Every operation has one true safety ground, every safe public caller is adversarially reviewed, and no allowance grows silently |
| PW-507 | READY | P2 | Documentation checking does not cover Markdown links/anchors, CLI claims, all numeric claims, or docs 008-012 from D-146 | Add cheap structural checks and complete the manual semantic/numeric sweep without rewriting historical results | Broken link/anchor/command/current-count fixtures fail; corrections distinguish dated history from current state |
| PW-508 | READY | P2 | A release lacks one reproducible evidence manifest tying source, tools and booted bytes together | Add a release/evidence task with source commit, locked dependencies, firmware/vendor hashes, kernel/image hashes and test results | Two clean paths produce byte-identical artifacts or an ADR names each unavoidable input; the manifest verifies offline |
| PW-509 | READY | P2 | QEMU machine aliases, firmware, VMware, Java and Rust versions can drift underneath evidence | Pin where stable; otherwise record exact resolved versions and hashes per qualification run | Re-running from the manifest either recreates the environment or reports the precise unavailable dependency |
| PW-510 | DECIDE | P2 | D-16 deferred advisory/license/SBOM policy, yet the project now vendors boot and firmware artifacts and ships dependencies | Choose advisory, license inventory, SBOM and offline-build expectations proportionate to a solo OS | Tooling is self-contained or deliberately exceptional; a dependency/license/digest fixture makes the check fail |
| PW-511 | READY | P2 | No stable performance/resource baseline says when a correctness-preserving change regresses the foundation | Record deterministic image size, unsafe count, boot time bands, frame/heap/handle steady state, trap latency and storage throughput | Repeated debug/release/QEMU runs state variance and thresholds; baselines are evidence, not ABI promises |
| PW-512 | READY | P2 | QMP probe/rebind has a local port race and QEMU argument handling has unproved comma/path edge cases | Remove the race where practical and test paths containing spaces/commas plus concurrent runners | Two simultaneous runs cannot steal control channels; hostile temporary paths reach the intended image |
| PW-513 | READY | P2 | There is no supported Windows debugger task, slowing the exact boundary work still open | Decide LLDB/GDB route and add a documented non-destructive `xtask` entry if a viable pinned tool exists | A breakpoint at kernel entry and one symbolized backtrace work from a clean documented setup |
| PW-514 | READY | P2 | One real `redundant_clone`, stale historical prose and a stale i8042 port-claim comment are already named | Remove the lint and correct current-facing prose without upgrading old evidence | Clippy/prose tests pass and dated historical numbers remain explicitly historical |

## Compiler gate and repository handoff

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| WZ-001 | WIZZY | GATE | The inspected compiler revision was local-only | Publish/reachably pin one `wizards-ink` revision, toolchain and every dependency | A clean machine can fetch exactly the recorded inputs; no sibling working tree is implied |
| WZ-002 | WIZZY | GATE | Wizzy has no non-interactive standalone program artifact | Add one documented command from standalone source to a versioned loadable artifact | Artifact identifies/rejects versions and runs without compiler, checked AST, compiler maps or host services |
| WZ-003 | WIZZY | GATE | The VM/runtime retains compiler structures and uses `std` host facilities | Separate the execution runtime behind a host-independent platform seam and build it with `no_std + alloc` | Target runtime dependency audit contains no compiler/AST/host I/O; host and minimal target adapter run the same fixture |
| WZ-004 | WIZZY | GATE | Reproducible compiler output is not established | Lock inputs and make two clean builds byte-identical | Gate record includes both commands, environments and matching artifact hashes |
| PW-600 | READY | GATE | The shared handoff is described but has no single machine-readable version record | Specify target ID, artifact/runtime/platform ABI versions, effect requirements, provenance and digest schema without choosing a delivery route | Parser/fixture rejects unknown/mismatched versions and accepts a pinned example independent of a sibling checkout |
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

## Audit cautions and prose cleanup handoff

The static audit deliberately did not promote these observations into P0
claims: NVMe polling is bounded (the defect is DMA lifetime after its timeout);
the final ring-3 crossing already refuses a non-present, non-user or
non-executable ELF entry (PW-109 is earlier/cleaner refusal); and the two
`map_shared` callers happened to keep their backing alive, so PW-100 was an
unsafe *safe API* rather than a demonstrated exploit before D-185 closed it.
NX is present on the current QEMU/VMware targets; PW-301 is the unrecorded
unsupported-hardware policy.

PW-514 includes these exact stale-current-facing corrections still to make:

- `011-userspace.md`'s old known-hole list still names leaf-closure and
  `MappedStack` defects closed by D-121 and D-125; only the leaked spin guard
  remains there, while D-141's active-space door should be named;
- `011` still says `SPINLOCKS_HELD` is global (D-154 made it per-CPU), bundles
  the now-paid timeout with the uncalled multi-queue wait, and can be read as
  claiming a first-class MemoryObject/dynamic stacks that do not exist;
- `009-multitasking.md` still calls timeouts and guarded boot/double-fault
  stacks open although D-164, D-89 and D-92 paid them;
- `001-boot.md` calls initrd module loading future although Phase 5 ships it;
- `012-research-notes-4.md` calls the backslash key unanchored although D-162
  closed it; and
- `000-vision.md`/D-03 still schedule an own bootloader while `010` explicitly
  declines it. That is a real decision contradiction: resolve it with a small
  ADR rather than silently editing history.

## Recommended execution order

1. Verify PW-201 on VMware before trusting a polished-kernel claim; PW-100,
   PW-101, PW-102, PW-103, PW-104, PW-105, PW-106, PW-107, PW-109, PW-200, PW-202, PW-203, PW-204, PW-205, PW-206, PW-207, PW-208, PW-209 and PW-300 are closed by D-185 through D-200.
2. Make spawn, teardown, degraded boot/time and hostile-input paths fail-atomic
   (the remaining P1 rows).
3. Harden the verification runner first (PW-500 through PW-509), so later
   evidence survives interruption and identifies exact bytes.
4. Run the complete closing evidence PW-001 through PW-006 on one commit.
5. In parallel but in `wizards-ink`, complete WZ-001 through WZ-004. Only then
   execute PW-601/PW-602 and begin `015`'s post-gate slices.

This ordering is a risk order, not authorization to skip the register's proof
rules. Every implementation still gets its check, every new check is broken
once, and every machine check gets a persistent mutation.
