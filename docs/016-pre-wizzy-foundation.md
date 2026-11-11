# 016 - The finite pre-Wizzy kernel foundation register

This is the handoff and the only current backlog for work in the Bedrock kernel
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
are **not closed**. `014-storage.md` owns the source totals. Historical green
evidence remains historical. PW-001 records each complete sweep report and its
counts. Every failed report is repair input, not qualification (D-301, D-302
through D-322, D-422). D-425 made the next qualified close a release rather
than a foundation close. D-441 supersedes only its first-v0.1.0 assignment:
that publication is an experimental source-only prerelease under the bounded
source-preview gate, is not the qualification campaign under What is left,
closes no rows, starts none of the stopped product work, and establishes no
current machine qualification. What is left gives the gate and order of a
qualified diagnostic release. v0.2.0 (D-449) is that qualified release: it
qualified `694061f` and closed every row its gate proved, leaving PW-006,
PW-010 and the `TRIGGER` rows live. The adversarial re-audit's safe-boundary,
hostile-layout and qualification defects are all recorded below. VMware waits
for a frozen candidate after the non-machine source and manual-red gaps close;
its fresh boots are themselves required to close the machine-dependent rows.

D-366 transfers compiler-readiness authority to Realm. The kernel handoff in
`015-wizzy-userland.md` records the last admitted evidence as **CLOSED**.
Closing this register does not open Realm's gate. WZ rows are external Ink
dependencies; product/post-gate references now name Realm-owned callers.
Their historical IDs and states remain intact, and kernel prerequisite work
still requires the named trigger. The 2026-10-07 rename is a source migration,
not a foundation close; Q's evidence does not qualify its successor.

The kernel foundation closes only when:

1. every `REQUIRED`, `READY`, `VERIFY`, and `DECIDE` row below is changed to
   `DONE` or `SUPERSEDED`, with its proof or superseding ADR linked in the row;
2. the phase-close evidence binds one exact qualification commit and, when
   used, its exact documentation-only closing child under D-330, and records CI, the full
   current mutation sweep, model checking, the advisory review and both VMware
   images, with the independent `xtask` grade and the Deep security scan either
   bound to that source or declared `deferred` (D-425);
3. every admitted P0 safe-boundary/device defect is closed.

That closes the kernel foundation; it does not authorize Wizzy work here.

## Status after the 2026-09-11 complete sweep

What remains, and exactly how to run it:

1. **PW-001, the registry sweep.** `cargo xtask mutate` from a clean
   checkout. Repeat the complete current registry, whose size the 014 closing
   table states, on a clean qualification commit. The
   runner journals every outcome and resumes after interruption, and 292 of its
   entries boot QEMU, so do not run anything CPU-heavy beside it. Its pass supplies the current-source rerun for every
   still-live row whose close requires a retained mutation; the register below,
   rather than a duplicated range summary, is authoritative.
2. **The `xtask` grade, a continuous campaign.** D-425 supersedes PW-025
   as a release prerequisite; a grade record enters only the release whose
   exact source it names. The 26 host libraries are graded and adjudicated
   (D-246). On a frozen `main` commit, put D-342's reviewed `cargo-mutants
   27.1.0` executable and D-324's pinned `cargo-nextest 0.9.145` on `PATH`,
   review every exact non-caught disposition in the tracked UTF-8
   `xtask/mutants-adjudications.json`, then run `cargo xtask
   qualify-mutants-grade --run`. `xtask` sets the one-worker environment,
   reserves the private output directory, and owns the exact cargo-mutants
   process tree through the bounded runner. Every candidate rebuilds the
   package, and a viable mutant may run its full host suite. A two-mutant probe
   took 158.9 seconds, so budget several days and do not run another
   memory-heavy qualification beside it. A pass additionally runs the independent `--list --json --diff
   --no-times` inventory and `cargo test -p xtask --doc --locked`; the current
   required doctest result is zero doctests. There is no caller-supplied
   result directory or seal action (D-373). D-429 matches the live exit to the
   completely parsed and adjudicated counters: timeout requires 3, otherwise
   missed requires 2, otherwise 0. The empty tracked policy is not a completed
   review: retain a discovery campaign, review its actual non-caught outcomes,
   commit the exact policy, and rerun on that frozen source before claiming a
   grade.
3. **VMware.** PW-003, PW-004, PW-201, PW-301, PW-305 and the hardware half of
   PW-534 need Isaac's boots: `cargo xtask vmware` and `cargo xtask vmware
   --shipped`, then `cargo xtask qualify-vmware` on each fresh transcript.
4. **Evidence and release.** With the sweep, both VMware qualifications,
   advisory and model records present on the qualification commit, run `cargo
   xtask evidence` and `cargo xtask verify-evidence` on that same commit and tag
   it directly (D-425). The `xtask` grade and owned Deep records enter the v11
   bundle only when present for that source; otherwise the manifest declares
   each `deferred`. The commit after the tag closes PW-009, the bundle clause of
   PW-028 and every other row the gate proved, and lists the remaining live rows
   as known open items. D-330's documentation-only child remains supported but
   is no longer the planned close. The continuous Deep campaign keeps D-406's
   authenticated invocation channel: run `cargo xtask qualify-security-deep
   --run` alone on a frozen `main` commit. D-416 makes that owned dialogue
   perform exactly one non-authoritative Daybreak access advisory before Deep
   start; its result neither grants model access nor controls PASS. D-417
   advances only the exact remeasured Node, Codex, originless-default and
   generated-system-skill tuple, and D-419 binds the verifier's one ordinal
   digest. D-420 establishes the owned clone and the intended root-deny MXC
   worker boundary. Because pinned Codex Security 0.1.32 requires a
   root-readable managed provider envelope, D-422 keeps that exact envelope for
   the authenticated app-server control plane, which may perform only
   configuration, account and API orchestration, and attenuates every initial
   or resumed exec profile exactly once to the strict root-deny worker profile,
   which disables network. No latest-tool discovery,
   alternative digest, weaker backend, wrapper bypass or ambient sandbox
   selection is allowed. D-422's v3 transition proof and D-424's preflight
   probes bind every reported execution thread and denied operation; they do
   not claim that the proprietary model selected each negative operation, and
   the accepted route is not qualification evidence yet. Fix every reportable
   finding, and repeat on a new frozen commit until the owned canonical scan
   passes. PW-010 is the census synchronization that the phase-close ADR binds.

The compiler gate stays **CLOSED**; nothing above touches it.
WZ-001 through WZ-004 are externally owned compiler-gate blockers in
`wizards-ink`. Only after they pass together at one reachable revision may a
separate gate-opening ADR record the pins, commands, hashes, runtime seam and
handoff version. Product integration remains stopped until that ADR; the only
pre-gate consumer exception is PW-601 after its exact pinned-draft/runtime
trigger fires.

`TRIGGER` is not unfinished work. It means **do not build this yet**; the row
records the exact event that can move it into the live register. Post-gate and
later-phase product work is classified at the end and is not part of kernel
completion.

## Register vocabulary

| State | Meaning |
|---|---|
| `REQUIRED` | Evidence required to close the current phase; run it on the qualification commit, which D-425 tags directly, with only D-330's optional exact documentation child afterward |
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
| PW-001 | DONE | P0 | Complete sweeps on `f28428a` (1,359 entries: 1,301 caught, 34 survivors, 24 inconclusive), `44e1c89` (2,051 of 2,062), `0dfcb76` (2,132 of 2,133), `20cf580` (2,222 of 2,228) and `62ee03a` (2,411 of 2,412) failed; every failed report is repair input rather than qualification | D-301 makes the first report repair inventory; D-302 through D-322, D-122, D-370, D-407, D-422 and D-447 repair the non-caught outcomes, keeping permanent mutations 2,036, 2,202 and 2,432. The 2026-10-19 receipt repair asserts restricted network in both sandbox states, binds the applied profile and the provider and effective applied-digest mapping, and covers the whole denied-write probe list; mutation 2,202 now removes the canonical-render comparison. Repeat the complete updated registry from a clean qualification commit | The complete 2,412-entry sweep on `694061f` (report sequence 2) caught every mutation through its named witness with no survivor or inconclusive outcome, including permanent mutations 2,036, 2,202 and 2,432; the verified evidence bundle binds that passing report (D-449, v0.2.0) |
| PW-002 | DONE | P0 | The ring model is a phase-close gate outside CI, and its last green result predates this adversarial repair series | Re-run `cargo xtask model-check` on the qualification commit | `cargo xtask model-check` passed on `694061f`: both ring models ran clean and every model mutation violated exactly its invariant; the source-bound record is in the verified bundle (D-449, v0.2.0) |
| PW-003 | DONE | P0 | The dated VMware result predated D-181/D-182, and D-421 and D-426 found verifier fields and order that would refuse a genuine boot | D-426 orders the self-test list as the 2026-10-20 boot emitted it and requires the PIIX4 quiesce record; boot `cargo xtask vmware` with every keyboard round | Two fresh VMware self-test machines of `694061f` passed 294/294 with every keyboard round and `qualify-vmware` accepted both (D-449, v0.2.0) |
| PW-004 | DONE | P1 | A self-test image is not the image users boot, and its shipped list was out of kernel order | D-426 orders the shipped list and D-427 adds a saved-CPL3 call observation required by both shipped runners; boot `cargo xtask vmware --shipped` | The shipped VMware machine of `694061f` emitted exactly one `user: ring 3 call observed` and `qualify-vmware` accepted it; shipped QEMU passed in CI (D-449, v0.2.0) |
| PW-005 | DONE | P1 | The ordinary gate must describe the evidence source, including D-330's exact documentation-only child when used | Run `cargo xtask ci` on the frozen qualification source and after the documentation-only close | `cargo xtask ci` passed on `694061f`, the commit D-425 tags directly; no D-330 documentation child was used (D-449, v0.2.0) |
| PW-006 | REQUIRED | P1 | D-181 and D-182 were post-audit corrections, and this audit found deeper safe/device boundaries | Re-attack hostile inputs, cleanup paths, safe `wost` callers and release-only invariant failures after the P0/P1 fixes | Phase-close ADR links the audit, every break-test, full evidence manifest and remaining trigger-only rows; no historical evidence is invented |
| PW-007 | DONE | P0 | `last-run` held no overall verdict, and prose accepted an older sweep after a newer one failed | D-244 writes immutable sequenced reports with verdicts and holds the sweep lock while publishing | 2026-08-30 host runs kept a newer failure beside an older pass, refused stale input and proved the lock |
| PW-008 | DONE | P1 | The model-mutant observer accepted any nonzero exit with the invariant text and had no deadline | D-244 requires one exit-consistent TLC classification, an exact trace shape, a deadline and a capture cap | 2026-08-30 hostile transcripts were rejected and a parked child proved bounded return |
| PW-009 | DONE | P1 | No clean closing run had produced and replay-verified the complete bundle under D-330 and D-425 | Create, verify and tag evidence on the frozen qualification commit (D-425); D-330's documentation-only child remains supported | `cargo xtask evidence` and `verify-evidence` passed on `694061f` with the `same-commit` relation, reproducing kernel and image byte for byte (D-449, v0.2.0) |
| PW-010 | READY | P1 | The staged PW-006 audit miscounted hostile surfaces and machine checks, omitted ring submissions and treated absence of explicit panics as panic freedom | Keep the [hostile-surface census](017-pre-wizzy-hostile-surface-audit.md) synchronized with every arithmetic, indexing, shift and callee boundary; D-325 checks omitted live rows and stale explicit `OPEN` or cross-cutting live classifications, and D-341 refuses a second `OPEN` segment, an `OPEN` label outside the proof column or an unreadable cross-cutting row | The phase-close audit accounts for every classified surface and links each independent oracle or explicit trigger |
| PW-011 | DONE | P1 | Framebuffer row copying stalled boots, and a raw-copy shortcut bypassed checked MMIO without D-41's measurement | D-249 moves row-span arithmetic to `libs/pixel` behind checked volatile MMIO; D-329 and D-332 bind the chain and witnesses | On 2026-09-28 retained mutations 841 through 844 each failed only its named row-plan witness, and two restored debug boots passed 262/262 |
| PW-012 | DONE | P1 | Model qualification discarded all six TLC outputs, so offline evidence verified only an assertion | D-262 retains, binds and replays one bounded transcript per registered run | 2026-08-31 fixtures and persistent mutations rejected omitted, altered and underived transcripts |
| PW-013 | DONE | P1 | The bounded tool helper did not contain delegated process trees, so timed-out witnesses left descendants | D-347 runs every bounded command in a Windows job object; non-Windows process groups stay with PW-708 | On 2026-09-28 mutations 1,522 and 1,523 failed their fixture, and 1,521 is retired with the `process-wrap` wrapper it broke |
| PW-014 | DONE | P1 | Evidence readers loaded attacker-sized files whole before refusing them, risking verifier memory | Stream hashes, require exact image sizes and cap every manifest, record and transcript before allocation | Oversized and sparse fixtures are refused before payload reads, and each size-door mutation reds its witness |
| PW-015 | DONE | P2 | `update-sbom` wrote through the existing SPDX path, so a planted link could truncate an outside inode | Stage with `create_new` in the validated directory, sync, revalidate and publish by replacement | Link sentinels stay byte-identical; concurrent local pathname replacement remains an explicit same-authority race rather than a claimed guarantee |
| PW-016 | DONE | P1 | `cargo xtask clean --all` could follow a swapped `vm/` ancestor even after D-263 (Codex Security occurrence `occ_b5eed8ca35b5d6b6379fb621`), and Windows locked its own `xtask.exe` | D-382 empties `vm` through retained no-follow handles within depth and entry budgets; D-418 admits only the sole real current process image as the Windows residual | Mutations 794 through 796, 1,773 through 1,775 and 2,086 through 2,092 each red their witness; outside sentinels stay byte-identical |
| PW-017 | DONE | P1 | QEMU's NVMe builder wrote its backing path directly and could lose the prior namespace | Validate real roots, stream into a `create_new` stage and keep rollback authority until publication validates | Link sentinels stay byte-identical and a late refusal restores the prior image; the no-directory-handle race remains explicit |
| PW-018 | DONE | P1 | VMware qualification accepted required text inside warning, prefixed or mixed records | Bind each requirement and the verdict to one severity-checked canonical record per ordered step | Hostile transcripts are refused for both selectors, and weakening the record shape reds the parser witness |
| PW-019 | DONE | P1 | QEMU teardown could block in `wait` past every advertised boot timeout | Stop and reap the child through a named polling deadline with no blocking wait | Delayed, refused and racing exits return at the bound, and deleting either refusal reds the witness |
| PW-020 | DONE | P1 | VMware generation used truncating writes, so a planted link could modify an outside inode | Create every output with `create_new` and validate the selector before writing its receipt | Planted link sentinels stay byte-identical; the no-directory-handle race remains explicit |
| PW-021 | DONE | P1 | API-audit copied source through truncating writes, so a planted link could modify an outside inode | Validate each real workspace component and copy into `create_new` files | Link sentinels stay byte-identical and the audit refuses before compiling; the no-directory-handle race remains explicit |
| PW-022 | DONE | P0 | A retained HostTest had an unbounded capture, could count a timeout as caught and could leak a reservation | Give each host-test child a deadline, post-kill poll and capture cap; control failures are inconclusive | Parked, noisy and delayed fixtures return within bounds, and each mutation fails its witness; tree containment is PW-013's |
| PW-023 | DONE | P1 | Boot-image construction truncated `target/wizard-os.img` directly and could lose the prior image | Build in a same-directory `create_new` stage and publish only beneath the real target with rollback | Link sentinels stay byte-identical and failures keep the prior image; the no-directory-handle race remains explicit |
| PW-024 | DONE | P0 | Mutation snapshot and restore acted before proving containment and could strand the workspace source | Validate every destination beneath the canonical root first and keep the old source as rollback authority | Redirect fixtures leave outside state unchanged and a late refusal restores the old source; the same-authority race remains explicit |
| PW-025 | SUPERSEDED | P1 | The project-authored registry and checks can share blind spots, and `xtask` lacked an independent grade | D-373's owned `qualify-mutants-grade --run` over `xtask/mutants-adjudications.json`, with D-340, D-342 and D-429; actual campaign adjudication and completion remain outstanding | Superseded by D-425 (2026-10-19): the grade is a continuous non-blocking campaign whose record enters only the release whose exact source it names, and a bundle otherwise declares it `deferred` |
| PW-026 | SUPERSEDED | P1 | No variance-reducing multi-pass Codex Security result was bound to the qualification source | D-406's owned `qualify-security-deep --run`, repaired by D-416, D-417, D-419, D-420, D-422 and D-424; D-373 proves unsigned provider artifacts cannot carry PASS authority | Superseded by D-425 (2026-10-19): Deep is a continuous non-blocking campaign whose record still requires the real owned run, because implementation and configuration witnesses, a failed mutation run, the advisory, and `--begin` or `--verify=DIR` are not Deep evidence; a bundle otherwise declares it `deferred` |
| PW-027 | DONE | P1 | D-141's condition fired when D-182 added trap-served Duplicate | D-255: `Arena::claim` selects and fills under the sole global guard, so no split window exists | Removing the install makes `filling the arena never repeats a slot` reuse a slot; the mutation is retained |
| PW-028 | DONE | P1 | No fresh RustSec run was bound to the source, and `cargo-audit` 0.22.2 names its database only when fetching | D-297 binds the pinned verdict to the fetched revision and checkout identity (D-274, D-220) | `cargo xtask qualify-advisory` passed freshly on `694061f` with no vulnerability or warning (D-449, v0.2.0) |
| PW-029 | DONE | P0 | The degraded watcher stopped at the first panic line, and the UART dropped timed-out bytes | D-260 emits two framed v1 panic records, propagates UART refusals and treats a truncated record as inconclusive | Host oracles reject each framing defect; on 2026-09-27 mutation 569 produced the exact `[error] panic: terminal record complete v1` and restoration settled |
| PW-030 | DONE | P1 | The sweep stopped because Git's raw path order and `PathBuf` order disagree | D-298 validates order and uniqueness on Git's NUL-delimited bytes before path conversion | 2026-09-10 the sweep on 05412f8 reproduced the refusal; mutation 1,380 retains the defect |
| PW-031 | DONE | P1 | The workspace's verbatim Windows `CARGO_TARGET_DIR` broke the MinGW linker | D-299 converts only verbatim disk or UNC spellings at the shared Cargo driver boundary | 2026-09-10 the disk and UNC fixtures passed, and mutations 1,067 and 1,381 retain both defects |
| PW-032 | DONE | P1 | The model-digest test borrowed the live Git index, so D-208's mutation copy failed its baseline | D-300 gives the fixture its own temporary Git repository | 2026-09-10 the baseline on 1ddef80 reproduced the failure; mutation 896 retains the defect |

## Safe memory, paging and resource lifetime

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-100 | DONE | P0 | Safe `AddressSpace::map_shared` let `Shared` die while its mapping survived | D-185 makes the space hold an allocation keepalive until proven reclaim | 2026-08-27 QEMU `a shared mapping keeps its frame` passed and dropping the keepalive reddened it |
| PW-101 | DONE | P0 | `map_range` rollback could clear an identical mapping that predated the failed call | D-185 preflights each transaction as wholly absent or wholly identical | 2026-08-27 QEMU `a refused remap preserves every older entry` passed and ignoring the preflight reddened it |
| PW-102 | DONE | P0 | `mem::with_active_space` gave every safe module `&mut AddressSpace` over the live tables | D-189 keeps the borrow private to `mem`; production exposes only stack and device mapping | 2026-08-27 exporting the closure reddened the host test; D-353 later replaced its name search (PW-563) |
| PW-103 | DONE | P1 | Any failure after `AddressSpace::for_user` in `user::spawn` could abandon resources | D-191 keeps the never-run process in one rollback transaction through reserved-slot commit | 2026-08-27 QEMU `every refused spawn returns every resource` restored exact baselines across all fourteen edges |
| PW-104 | DONE | P1 | A space tracked only 32 table frames, and refused cleanup read as zero returned | D-190 grows table ownership fallibly and makes refusal a typed, logged result | 2026-08-27 QEMU `a large space returns every table frame` returned all 37 frames |
| PW-105 | DONE | P1 | A safe `mem::forget` of a `SpinLockGuard` strands lock and interrupt state | D-194 bounds deliberate guard loss as a liveness violation and keeps every D-66 refusal; shipped code gets no unsound recovery door | 2026-08-27 suppressing the transition reddened only `forgotten lock ownership is distinct from contention` |
| PW-106 | DONE | P1 | Queue teardown consumed a pending operation whose completion could not post | D-192 retains a settling operation until its terminal record posts and the zombie until its ring is quiet | 2026-08-27 deleting restoration reddened only `a refused settlement retains every owner` |
| PW-107 | DONE | P1 | A failed guarded boot stack still let ring 3 launch on the unguarded stack | D-193 refuses process construction unless the boot stack is guarded | 2026-08-27 bypassing the gate reddened only `an unguarded boot stack closes the user boundary` |
| PW-108 | DONE | P2 | The copied boot memory map silently capped evidence at 128 regions | D-224 moves the map into `libs/memory-map` and refuses an oversized handoff whole | 2026-08-29 restoring silent overflow reddened only `the_region_limit_is_all_or_nothing`, retained as mutation 497 |
| PW-109 | DONE | P2 | The ELF entry was refused only after allocation, and `PLACED` was global state | D-191 validates the entry in preflight and returns placements in `LoadedImage` | 2026-08-27 QEMU refused an outside entry with no frame change; bypassing preflight reddened the loader check |
| PW-110 | DONE | P0 | `AddressSpace::map_boot` accepted `USER`, so a kernel or firmware frame could reach ring 3 | D-243 makes every boot-evidence mapping supervisor-only | 2026-08-30 removing the guard failed only `a boot mapping cannot publish kernel memory`, retained as a mutation |
| PW-111 | DONE | P0 | A raw root could mint `Released` while an uninstalled `Binding` lived | D-243 makes installation return a linear `Bound`; only the uninstalled binding or a dead bound token releases | 2026-08-30 accepting the live release failed only `a bound space is not reclaimed` |
| PW-112 | DONE | P0 | `MemoryMap::covered_by` skipped a forbidden region nested inside an allowed one | D-243 arbitrates every intersecting claim before walking the coverage union | 2026-08-30 an exhaustive byte oracle failed on the original jump and on the retained mutation |
| PW-113 | DONE | P0 | Safe map doors ignored live stack and guard pins, and pinning checked only the top page | D-259 protects every `guard_base..stack_top` extent and rewalks the loaded root; boot and double-fault pins stay irrevocable | 2026-08-31 release QEMU refused lower-leaf substitutions in all three stacks; both defects are retained mutations |
| PW-116 | DONE | P0 | `spawn` readiness could age before the ring-3 crossing, and refusal did not recover the owner | The final door rereads live readiness, `task::enter_user` refuses a pre-disabled caller, and refusals disable, orphan and exit | 2026-08-30 each defect failed only `unadmitted interrupts cannot be enabled by a user transition` |
| PW-117 | DONE | P0 | Dropped or refused user-byte owners could free user bytes without zeroing | D-331 zeroes before every natural external-byte free and withholds a run whose clear refuses | On 2026-09-28 four defects failed `external user bytes are zeroed before free` or `a refused external clear withholds its run` |
| PW-118 | DONE | P0 | PCI `CommandState` could be replayed, nested or used on another registry | D-335 makes command state a linear per-function lease consumed only through its issuing registry | On 2026-09-28 mutations 749, 750 and 757 through 759 each reddened its host witness (D-335) |
| PW-119 | DONE | P1 | Safe `invalidate_page(u64)` accepted noncanonical addresses, for which `invlpg` raises `#GP` | Admit only a checked canonical address and keep the instruction private | On 2026-09-27 mutations 777, 785 and 786 each failed only its oracle, contract or machine check |
| PW-120 | DONE | P0 | Stack and device liveness rewalked the recorded root while consumers used the current CR3 | D-337 binds liveness to the active root and revalidates extent and leaf identity before use | On 2026-09-28 mutations 817, 818 and 821 through 825 each failed only its stale-root or leaf check |
| PW-121 | DONE | P0 | APIC self-IPI and timer admission could accept the spurious vector | One `DeliverableVector` constructor proves source and spurious vectors architectural and distinct | Host tests exhaust all 65,536 pairs, and deleting the exclusion failed its witness |
| PW-122 | DONE | P1 | `set_timer_one_shot` could revector a live APIC countdown | `disarm_timer` returns a linear `DisarmedTimer` that `set_timer_one_shot` consumes after rechecking the count | 2026-09-14 an `if false` refusal failed only `refused_one_shot_plans_perform_no_fake_register_write` |
| PW-123 | DONE | P1 | `claim_eoi(vector)` could mint two live `EoiToken`s for one delivery | D-336 latches one live claim before minting and releases it only on completion or drop | On 2026-09-28 mutations 736, 773, 774 and 1,174 each failed its machine, panic or compiler witness |
| PW-124 | DONE | P2 | Release `wost` exposed self-test-only probe doors | The probes compile only for self-test, and a compiler contract requires their absence when shipped | 2026-08-30 removing the cfg reddened only `contracts::machine_probes_are_absent_from_the_shipped_wost_graph` |
| PW-125 | DONE | P1 | `wost::pci::probe_bars` kept an uncertain reservation with no retry authority | D-335 returns typed restoration authority that is retried or terminalized, never silently released | On 2026-09-28 mutations 787 and 792 failed `an uncertain bar restore retains exclusion and retries exactly` |
| PW-126 | DONE | P0 | `apic::init` published access early and accepted the SVR enable without readback | D-336 publishes readiness last after the exact SVR readback and poisons on late refusal | On 2026-09-28 mutations 771 and 850 failed their checks; D-313 and D-257 retain mutations 849 and 851 |
| PW-127 | DONE | P0 | `paging::activate` published `ACTIVE` before CR3 with interrupts allowed | D-268 makes activation a `Cold -> Initializing -> Ready` transaction under interrupt exclusion that publishes `Ready` last | 2026-09-28 mutations 778 through 784 each reddened its witness (D-334) |
| PW-128 | DONE | P0 | Publication built an `AtomicU32` over ring-3 bytes, and shared aliases could pick a conflicting cache policy | D-269 gives each allocation one access layout; D-282 publishes by integer address, and D-283 refuses non-write-back flags | Caught in the complete 2,412-entry sweep on `694061f`; both fresh VMware self-test qualifications exercised the real ring (D-449, v0.2.0) |
| PW-129 | DONE | P0 | Safe PCI command and BAR-probe doors accepted a raw `Address` and a caller BAR count | D-335 requires one opaque linear function authority for every configuration write, with the BAR count from the verified header | On 2026-09-28 mutations 1,010 through 1,015 reddened the contract; D-343's door inventory is retained by mutations 1,453 through 1,481 |
| PW-130 | DONE | P0 | I/O APIC routing trusted accepted MMIO writes and unmasked before proving the destination | D-276 makes masking two-phase: the inert low word reads back before the destination changes | 2026-09-04 restoring the old order failed only the route-rollback witness, retained as mutation 839 |
| PW-131 | DONE | P0 | Root identity could keep a stale translation or evict an unrelated one when clearing | D-261 walks the actual CR3 before every clear and issues `invlpg` iff it reaches the identical slot | Release QEMU passed 252/252, and both defects remain retained mutations |
| PW-132 | DONE | P0 | Fresh tasks restored a zeroed XSAVE area, and user admission trusted remembered state | D-338 makes the initial image explicit, requires D-82's complete policy and uses one live predicate at both user doors | On 2026-09-28 mutations 431, 432, 717, 310, 1,170, 828 and 829 each failed its exact check; D-333 adds the final-door red |
| PW-133 | DONE | P1 | Four unsafe instruction doors were public beyond their compiled callers | Narrow CR3 and XSAVE doors to the architecture module and make raw FS-base writing private | On 2026-09-27 mutations 1,161 through 1,169 each failed only `unsafe_instruction_doors_match_their_compiled_caller_scope` |

## PCI, NVMe, FAT and filesystem resilience

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-200 | DONE | P0 | A timeout could release buffers or controller state while the device might still DMA | D-188 requires a verified Controller Reset before release, otherwise `DmaSegment` stays inaccessible; D-232 and D-285 strengthen retirement | 2026-08-27 a delayed fake completion stayed retained after an unverified stop |
| PW-201 | DONE | P0 | D-187's restored live BAR probe still needed its VMware observation | Run PW-003 against D-187's transaction and record the discovered NVMe BAR | Both fresh VMware self-test qualifications of `694061f` record both NVMe BARs below MAXPHYADDR 45 before `nvme: up` (D-449, v0.2.0) |
| PW-202 | DONE | P0 | PCI claims keyed the offset-bearing address, committed early and could strand ownership | D-187 canonicalizes BDF identity and makes preparation and I/O windows all-or-none | 2026-08-27 the canonicalization, early-commit and partial-port mutations reddened their checks |
| PW-203 | DONE | P1 | `read_blocks` admitted zero, overflowing and unproven DMA lengths | D-195 accepts only a validated `libs/nvme::ReadTransfer` | 2026-08-27 the bypass reddened only `invalid nvme reads stop before submission` |
| PW-204 | DONE | P1 | Namespace Identify trusted hostile format metadata | D-196 makes `libs/nvme::Namespace` prove extent and a plain 512-byte format before I/O queues | 2026-08-27 the bypass reddened only `invalid nvme namespace stops before io queues` |
| PW-205 | DONE | P1 | A ring-3 Read polled the controller inside its trap | D-200 queues validated Reads to one eight-slot preemptible worker that owns the controller | 2026-08-27 release QEMU kept IRQs progressing and refused the ninth request with `STATUS_NO_ROOM` |
| PW-206 | DONE | P1 | A bus-master enable failure still started NVMe | D-339's typed BME door retires mapping, queues and lease on refusal and never reaches `nvme::start` | On 2026-09-28 mutation 56 failed only `a failed bus master admission stops before nvme start` |
| PW-207 | DONE | P1 | Early PCI failure skipped the tmpfs and devfs survey | D-198 makes PCI, the namespace survey and interrupt bring-up sibling boot steps | 2026-08-27 degraded QEMU still enumerated tmpfs and devfs after the PCI refusal |
| PW-208 | DONE | P1 | One broken NVMe controller masked a later good one | D-199 advances to the next candidate only after verified full retirement | 2026-08-27 the leak and stop mutations reddened `a later nvme controller survives an earlier failure` |
| PW-209 | DONE | P2 | NVMe timeouts were iteration guesses after a monotonic clock existed | D-200 gives `libs/nvme::PollLimit` a monotonic deadline, or an attempt bound in early bring-up | Host tests proved both bounds, and each defect reddened its test |
| PW-210 | DONE | P1 | The FAT oracle had two readers but one image writer | D-207 pins an independent dosfstools/mtools volume with verified provenance | 2026-08-28 both readers agreed, and two breaks persist as mutations |
| PW-211 | DONE | P0 | FAT32 accepted a short table and ignored the active-table selector | D-243 validates table capacity and implements mirrored and selected-active FAT | 2026-08-30 four breaks failed their tests and persist as mutations |
| PW-212 | DONE | P1 | NVMe PRP-list arithmetic could overflow near `u64::MAX` | Identify and Read share one checked contiguous-extent predicate | 2026-09-14 the wrap mutation failed `identify_binds_one_complete_aligned_page` and is retained |
| PW-213 | DONE | P0 | `DmaState` let safe cancellation return memory a device still owned | D-339 separates reservation from publication and makes both linear | On 2026-09-28 mutations 710, 711, 830 and 1,179 through 1,181 each failed its test or contract |

## CPU, time, interrupts and scheduling

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-300 | DONE | P0 | CR0.WP was neither enabled nor read back | D-186 puts WP in the boot protection contract with a private recovery window | 2026-08-27 omitting the bit reddened `supervisor writes obey read-only pages` |
| PW-301 | DONE | P1 | D-201's mandatory NX and calibrated progress before ring 3 still needed VMware evidence | Run PW-003 and record NX, calibration and successful user launch together | Both fresh VMware self-test qualifications of `694061f` record all three together (D-449, v0.2.0) |
| PW-302 | DONE | P1 | Failed calibration still launched user code and admitted Timer promises | D-201 gates user launch and Timer admission on live calibration | 2026-08-28 bypassing either gate reddened only `an uncalibrated clock admits no user or timer` |
| PW-303 | DONE | P1 | VMware refused calibration once, and rate change and revocation were unhandled | D-225 admits a complete invariant CPUID 0x15 ratio, keeps the PIT as validator and defines revocation; suspend remains unsupported | 2026-08-29 all six breaks persist as mutations 498-503 |
| PW-304 | DONE | P1 | ACPI kept only the first I/O APIC, so a later GSI owner was unroutable | D-205 inventories up to sixteen controllers and keeps the unique owner of each GSI | 2026-08-28 all five mutations reddened their tests, and release QEMU routed GSI 1 |
| PW-305 | DONE | P2 | Compacted `XSAVEC` restore still needed hardware observation | Run PW-003 and require the same x87/XMM/YMM-high switch check under the compacted `XSAVEC` banner | Both fresh VMware self-test qualifications of `694061f` pass `every enabled state component survives a switch` (D-449, v0.2.0) |
| PW-306 | DONE | P1 | The idle-start latch was set before spawn succeeded | D-202 reserves a three-state idle attempt and rolls back refusals | 2026-08-28 burning the reservation reddened only `a failed idle start can be retried exactly once` |
| PW-307 | DONE | P1 | A zombie was reaped only at idle or a later spawn | D-203 makes retirement wake the mandatory service worker | 2026-08-28 the missing-worker and missing-wake mutations each reddened only the reap check |
| PW-308 | DONE | P0 | XSAVE preflight accepted absent sub-leaves and AVX without SSE | D-243 rejects incomplete layouts and the AVX dependency before hardware writes | 2026-08-30 both rules failed their witness when deleted and persist as mutations |
| PW-309 | DONE | P1 | `Timebase::from_parts` admitted unpublishable multipliers and shifts above 63 | Raw construction refuses unpublishable multipliers and unevaluable shifts | 2026-09-14 both bound deletions failed their witness and remain registered mutations |

## Hostile boundary, concurrency and accounting

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-400 | DONE | P1 | Boundary code kept panic assumptions, and FAT geometry arithmetic could panic or wrap | D-206 guards target boundaries with source rules and validates FAT geometry at construction | 2026-08-28 five persistent mutations retain the refusals and rules |
| PW-401 | DONE | P1 | Shared-ring copies were ordinary memory operations with no compiler or SMP contract | D-221 makes each index the atomic acquire/release edge and payload copies volatile | 2026-08-29 each ordering break reddened its host witness and persists as a mutation |
| PW-402 | DONE | P1 | Table identity was a wrapping `u32` that could collide with live ownership | D-204 makes identity a non-repeating `u64` with typed exhaustion | 2026-08-28 the wrap mutations reddened only `table identities never wrap or repeat` |
| PW-403 | DONE | P2 | The handle mixer leaked one foreign-occupancy bit per probe | D-222 reports empty, reused and foreign slots alike as `Stale`; D-248 later removed close | 2026-08-29 the resolve break persists; D-248 superseded the close mutation |
| PW-404 | DONE | P2 | The models did not compose shared-memory visibility with terminal ownership | D-229 adds an identity-bearing weak-memory model and four bad configurations that must fail | 2026-08-29 each bad configuration violated only its invariant; mutations 523-525 retain the bindings |
| PW-405 | DONE | P2 | The syscall wake race and guard `!Send` property had no durable witnesses | D-223 drives a real child `RingEnter` and cross-compiles `Send` assertions | 2026-08-29 `a syscall wake arms the timer for what it woke` reddened; mutations 493 and 494 retain the type breaks |
| PW-406 | DONE | P1 | `wost::panic::reclaim` missed two static locks, risking a panic-path deadlock | D-293 gives each static lock a `force_unlock` door and derives the lock set from source | 2026-09-06 `contracts::every_static_lock_in_wost_is_reclaimed_by_the_panic_path` went from red to green |

## Verification, release engineering and repository truth

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-500 | DONE | P2 | `image::build` could confuse shipped and self-test images | D-211 makes `Kernel::{Shipped, SelfTest}` the complete image selector | 2026-08-28 the selector breaks reddened their checks and persist as mutations |
| PW-501 | DONE | P2 | CI composition and README counts were inferred from text | D-212 makes one typed registry own every graph and stage, rendered into help and README | 2026-08-28 two breaks reddened `the_ci_registry_drives_help_and_documentation` |
| PW-502 | DONE | P1 | The mutation runner edited tracked source in place | D-208 runs from a locked tracked-file copy with checksummed journals | 2026-08-28 killed runs resumed with source untouched; five breaks persist as mutations |
| PW-503 | DONE | P2 | Library break-tests were one-off and could stop reaching production code | D-210 keeps one production mutation per pure crate with a floor from `HOST_PACKAGES` | 2026-08-28 each new crate's defect reddened its test |
| PW-504 | DONE | P2 | A mutation could look caught when its check was already red | D-209 takes a green baseline and admits only the exact named check or panic | 2026-08-28 each break reddened only its host check |
| PW-505 | DONE | P2 | The public API sweep was a text search | D-213 adds a stable-rustc semantic reachability audit | 2026-08-28 29 dead APIs were removed and five defects persist as mutations |
| PW-506 | DONE | P2 | A total unsafe-line count cannot bind SAFETY prose | D-214 pins each unsafe-bearing file's reviewed revision under the 380-line budget; D-290 records 377 lines, 149 blocks, 39 declarations | The exact pin and budget checks pass at 377/149/39 |
| PW-507 | DONE | P2 | Documentation checks missed links, CLI claims and current counts | D-215 checks Markdown targets, parses every xtask command and derives current counts | 2026-08-28 each mutation reddened only its proof; mutations 1,365-1,370 retain the 2026-09-06 pass |
| PW-508 | DONE | P2 | A release had no reproducible manifest of source, tools and booted bytes | D-216 binds the gate, two locked builds and the booted artifact into an offline-verifiable manifest | 2026-08-28 commit `b88e752` verified offline, and nine refusals persist as mutations |
| PW-509 | DONE | P2 | Machine, tool and evidence schemas can drift under a qualification | D-218 pins the machine ABI; later decisions advance the formats to qualification v6 and evidence v11 | Mutations 2,262 through 2,271 retain D-425's checks, and source emits `wizard-qualification-v6` and `wizard-evidence-v11` |
| PW-510 | DONE | P2 | D-16 deferred advisory, license and SBOM policy | D-220 defines the lock-bound inventory, SPDX SBOM, paperwork and offline-build contract | 2026-08-29 every digest matched and SPDX regenerated identically; PW-028 owns the advisory result |
| PW-511 | DONE | P2 | No resource baseline caught a correct change that regressed the foundation | D-230 adds a bounded three-release and three-debug fixed-QEMU baseline | 2026-08-29 six boots held every band; mutations 531 through 536 persist |
| PW-512 | DONE | P2 | QMP and shared VM state raced under concurrent runners | D-227 gives every VM private firmware, image and port state | 2026-08-29 parallel boots passed; of mutations 512-519, 514, 515 and 517 are retired because D-346 removed their listeners |
| PW-513 | DONE | P2 | There was no supported Windows debugger task | D-231 pins WinLibs GDB 17.2 behind one private paused QEMU run | 2026-08-29 `cargo xtask debug --check` hit `wizard_kernel::kmain`; mutations 537 through 541 persist |
| PW-514 | DONE | P2 | A real `redundant_clone` and stale current-facing prose were named | D-219 removes the lint and names later decisions without rewriting dated evidence | 2026-08-29 mutation 463 fails only `current_facing_foundation_prose_names_the_later_resolutions` |
| PW-515 | DONE | P1 | The bootstrap-authority check paired a global count with a reused slot's exit | D-217 keyed exits to table identity; D-248 removed the pairing-only endpoint | D-288/PW-542 owns the current exact-incarnation correction |
| PW-516 | DONE | P1 | D-84 limited property testing to `libs/ring-proto`, so D-241's contradicting contracts went unseen | D-242 admits independently cited properties with stated reach and extends them to five crates | 2026-08-29 all five passed; mutations 549 and 550 persist |
| PW-517 | DONE | P1 | A sampled finite domain is a weaker claim than a walked one | D-242 admits bounded exhaustion, each test naming its bound | 2026-08-29 `libs/slab-layout/tests/exhaustive.rs` walked all 53,261 pairs |
| PW-518 | DONE | P1 | The off-by-one range mutation had no exact-red record | Run the mutation against `half_open_ranges_distinguish_crossing_adjacent_and_one_past` and restore | On 2026-09-13 only that property failed |
| PW-519 | DONE | P2 | The deleted `is_relative` predicate's mutation needed its exact red | D-328 requires the reintroduction to fail alone through the compiler-resolved graph | On 2026-09-27 `cargo xtask api-audit` named only `fn is_relative` |
| PW-520 | DONE | P1 | D-272's LF and HTTPS authority mutations lacked manual reds | Break each of the four boundaries one at a time, then restore | On 2026-09-13 each named `wizzy-handoff` test alone failed |
| PW-521 | DONE | P2 | The Shuttle harness's three mutations lacked manual-red records | Break each reservation transition and the admission guard, then restore | On 2026-09-13 `scheduled_calls_preserve_the_production_reservation` alone caught each |
| PW-522 | DONE | P2 | Zero-count MMIO semantics and the refusal oracle disagreed | D-273 makes an empty repeated access a no-op and compares every result class | 2026-08-31 three mutations each failed their witness |
| PW-523 | DONE | P2 | The deleted `TrapCause::is_resumable` method's mutation needed its exact red | D-328 requires the reintroduction to fail alone through the compiler-resolved graph | On 2026-09-27 `cargo xtask api-audit` named only `method impl::is_resumable` |
| PW-524 | DONE | P2 | The `x86-paging` `OWNED` diagnostic mutation lacked an exact red | Remove the owned formatter arm once and restore it | On 2026-09-13 only `tests::debug_says_what_the_entry_is` failed |
| PW-525 | DONE | P2 | The cleaned tree had not passed this row's complete gate | Run lints, decoder tests, shipped-observer contracts, the unsafe inventory and full CI | 2026-09-06 gate run 4 passed on the integrated tree |
| PW-526 | DONE | P1 | The frame-space bypass mutation lacked an exact red | Run the `is_managed` bypass against `release_authority_distinguishes_managed_in_range_unmanaged_and_out_of_range` | On 2026-09-13 only that property failed |
| PW-527 | DONE | P0 | Cleanup paths discarded refusals, and ring 3 opened without extended-state isolation | D-245 makes construction fail-atomic, returns resumable cleanup authority and closes user launch without XSAVE | D-245's dated release run restored exact ownership; exact breaks persist |
| PW-529 | DONE | P1 | D-85's `E0499` evidence was a one-time manual transcript | D-247 cross-compiles two real `SpinLock::lock`/`ThreadContext` refusals requiring `E0499` | 2026-08-30 removing `Drop for SpinLockGuard` failed the test |
| PW-530 | DONE | P1 | D-150 left direct-door wait ownership to prose review | D-247 requires one classification row and explicit arm per `Call` variant | 2026-08-30 deleting `RingEnter`'s classification failed the test |
| PW-531 | TRIGGER | WHEN-TRIGGERED | D-244's unique serial path and exact running-VMX observation prevent accidental replay, but the guest has no host-supplied per-generation challenge, so a privileged operator or filesystem can still copy an older valid transcript into the fresh path | Add process-to-transcript binding only when a concrete guest-readable per-generation challenge channel or platform attestation mechanism exists; do not describe today's local checksums as cryptographic provenance | Trigger: a selected guest challenge or attestation mechanism can carry the generation into the guest verdict; proof rejects a copied earlier transcript and binds the response to the exact VMX, image digests and observed run |
| PW-532 | DONE | P1 | Speculative IPC surface and test-only evidence sat in the shipped kernel | D-248 removes the surface and reclaims every raw space; D-290 confines observers to self-test | 2026-09-06 gate run 4 passed with the shipped-graph refusal contracts |
| PW-533 | DONE | P0 | Device paths discarded refusals and could drop their owner on cleanup | D-249 makes device transactions fail-atomic; D-285 keeps leftover owners in bounded quarantine | 2026-09-06 gate run 4 passed with the breaks registered |
| PW-534 | DONE | P0 | D-81 declined KPTI on an assumption while ring 3 launched with no `RDCL_NO` claim or QEMU proof | D-250 requires `RDCL_NO` or the exact QEMU TCG tuple for shared roots; D-407 permits a no-`RDCL_NO` processor only after an exact paired-root KPTI proof | Caught in the complete 2,412-entry sweep on `694061f`; VMware selected paired roots on the no-`RDCL_NO` target (D-449, v0.2.0) |
| PW-535 | DONE | P0 | D-277's vector door breaks needed exact red records | Keep the sole assignment and typed doors under contract and machine witness | On 2026-09-27 mutations 1,272 and 1,273 each failed only its named check |
| PW-536 | DONE | P0 | D-278 and D-285 BME and stop rules needed their retained breaks run | Run or bind the retained BME, quiescence, quarantine and rollback breaks; D-339 supplies PW-206's refusal | All 46 retained mutations, including mutation 56, were caught in the complete 2,412-entry sweep on `694061f` (D-449, v0.2.0) |
| PW-537 | DONE | P0 | D-278's BAR width, type and identity checks needed their breaks run | Run the retained BAR breaks and restore QEMU and VMware boots | Caught in the complete 2,412-entry sweep on `694061f`; VMware passes `a live nvme bar obeys maxphyaddr` (D-449, v0.2.0) |
| PW-538 | DONE | P1 | Safe `copy_from_user_unwalked` treated every lower-half page as user memory | D-279 removes the general door; production copying stays permission-walked | 2026-09-04 each break failed only its named check; PW-702 remains the concurrent-unmap trigger |
| PW-539 | DONE | P1 | One safe space could mint two live bindings | D-280 adds a machine witness and a production-owner inventory | On 2026-09-27 mutations 181 and 182 each failed only its named check |
| PW-540 | DONE | P0 | User roots copied higher-half entries before the device-window root existed | D-281 freezes the device-window top level; D-284 rejects huge or user entries | Caught in the complete 2,412-entry sweep on `694061f`, including mutation 326 (D-449, v0.2.0) |
| PW-541 | DONE | P0 | NVMe preparation overflowed the guarded 32 KiB boot stack | D-287 splits preparation into stages; D-326 keeps two pages free and D-361 paints the stack | `nvme preparation preserves two pages of boot-stack headroom` passes, and mutation 17 retains the break |
| PW-542 | DONE | P1 | Global ownership observations failed non-causally under refill and slot reuse | D-288 adds a conserved allocator oracle and exact zombie-incarnation protocol | On 2026-09-27 two debug and two release boots passed; mutations 1,323-1,334 retain the defects |
| PW-543 | DONE | P0 | CPL3 entry could leave live `SS` null for a kernel context | D-289 normalizes live `SS` at common entry before Rust | Caught in the complete 2,412-entry sweep on `694061f`; the first ring-3 call passed on VMware (D-449, v0.2.0) |
| PW-544 | DONE | P1 | D-275 rejected valid generic calls and could dispatch by name to the wrong implementation | D-291 resolves one exact Self and removes name-only dispatch; D-295 resolves `Type::from(value)` | Mutations 1,350-1,364 and 1,374-1,375 each reddened its fixture |
| PW-545 | DONE | P2 | The API audit ran over two and a half hours without finishing | D-292 builds `cargo xtask` optimized, and D-294 rebinds the registry once per round | 2026-09-06 the audit reached a verdict; a retained mutation reds the freshness test |
| PW-546 | DONE | P2 | Three public items had no caller: `BarAuthorityFailure::reason`, `::into_authority` and `BarBindFailure::into_range` | D-296 removes them, because an admitted device range is a CPU-derived proof rather than an owner | 2026-09-06 the complete gate accepted the tree after removal |
| PW-547 | DONE | P1 | Six public error enums violated C-GOOD-ERR | They carry standard errors, are non-exhaustive, and one syntax inventory enforces the rule | 2026-09-14 five retained mutations preserve the checks |
| PW-548 | DONE | P1 | Public failure wrappers could be discarded silently with their authority | Every public `*Failure` struct is `#[must_use]`, enforced by a syntax inventory | 2026-09-14 mutation 1,388 fails only `contracts::public_failure_wrappers_are_must_use` |
| PW-549 | DONE | P1 | `a thread crosses into ring three and calls back` flaked on global counts | D-327 binds the witness to the returned `(slot, table)` incarnation | Mutations 1,407 and 1,408 each failed only the crossing; three debug boots passed |
| PW-550 | DONE | P2 | Mutation numbers were array positions, so citations drifted unchecked | D-341 gives each mutation a permanent id and checks every living citation against the registry | On 2026-09-28 mutations 1,414 through 1,421 each failed once and passed restored |
| PW-551 | DONE | P1 | The `xtask` grade could not seal a real run | D-342 fixes the output path, runs inventory and doctest inside the seal and pins the grader | On 2026-09-28 mutations 1 and 1,422 through 1,426 each failed their host witness |
| PW-552 | DONE | P2 | Several D-329 and D-330 rules had never failed once | D-342 retains one defect per rule and removes two unreachable checks | On 2026-09-28 mutations 1,427 through 1,452 each failed their host witness |
| PW-553 | DONE | P2 | Source identity trusted whichever `git` came first on `PATH` | D-344 resolves Git once and binds its path, version and SHA-256 into every record | On 2026-09-28 mutations 1,482 through 1,496 each failed their witness |
| PW-554 | DONE | P2 | Checked paths were reopened with a link-following open | D-344 opens checked paths through one no-follow primitive, enforced by a syntax check | On 2026-09-28 mutations 1,497 through 1,500 each failed their witness |
| PW-555 | DONE | P2 | Contract probes used predictable shared temporary directories | D-344 creates probes exclusively under `target/contract-probes` | On 2026-09-28 mutations 1,501 and 1,502 each failed the witness |
| PW-556 | DONE | P2 | The supply-chain check misread sources, tables and notices | D-345 requires exact sources, refuses unmodeled shapes and requires exact notice entries | On 2026-09-28 mutations 1,503 through 1,509 each failed their witness |
| PW-557 | DONE | P1 | QMP and the monitor were loopback listeners any local peer could take | D-346 runs QMP over the emulator's own standard streams with bounded, deadlined messages | On 2026-09-28 mutations 1,510 through 1,520 failed their witness; mutations 514, 515 and 517 are retired with the listeners |
| PW-558 | DONE | P0 | User memory could reach the last canonical page, faulting `iretq` in ring 0 | D-348 makes that page never user memory through `USER_MAPPING_END` | On 2026-09-29 mutations 1,524 through 1,526 each failed their witness |
| PW-559 | DONE | P0 | Safe RAM mapping doors could give one physical run two cache types | D-349 maps boot ranges only with their own type and refuses cache types on owned and user maps | On 2026-09-29 mutations 1,527 through 1,530 each failed their witness |
| PW-560 | DONE | P1 | I/O APIC takeover trusted a version register from any device | D-350 accepts only versions 0x11 to 0x20 and at most 240 entries | On 2026-09-29 mutations 1,531 and 1,532 each failed their witness |
| PW-561 | DONE | P0 | Any kernel component could claim any device's ports | D-351 gives each known device a typed door and refuses raw claims in production code | On 2026-09-29 mutation 1,533 fails the contract |
| PW-562 | DONE | P1 | A cyclic FAT directory chain could stall boot | D-352 caps a directory at 65,536 entries and detects cycles on the cursor | On 2026-09-29 mutations 1,534 and 1,535 each failed their host test |
| PW-563 | DONE | P2 | Two validated test files were never landed, and D-189 rested on a name search | D-353 lands both tests and replaces the name search with a syn item inventory | On 2026-09-29 mutation 301 reddened the inventory naming `with_active_space` |
| PW-564 | DONE | P1 | Two fault witnesses could pass on another process's fault | D-354 binds each witness to the `Spawned` it launched | On 2026-09-29 mutations 1,536 and 1,537 failed only the named checks |
| PW-565 | DONE | P1 | The busy-reap witness could pass on another program's reap | D-355 binds it to its `Spawned` and persistent last rites | On 2026-09-29 mutation 1,538 failed only the named busy-reap check |
| PW-566 | DONE | P2 | Queue backstops were `debug_assert!` arms the release build elides | D-356: `kernel/src` holds no `debug_assert`, and every backstop is a terminal `panic!` or `unreachable!` | On 2026-09-29 mutation 1,539 reddened only its self-test |
| PW-567 | DONE | P0 | The storage controller could bus-master anywhere in physical memory (Daybreak findings 1 and 8) | D-357 and D-358 put each candidate behind a VT-d domain mapping only live segments | On 2026-10-03 both self-tests passed 270/270 with translation on; their mutations each failed their named check, and 1,563 is retired |
| PW-568 | DONE | P0 | Bus mastering was not bound to a remapping domain for its function | D-357 and D-358 make `admit_bus_master` take a `DmaDomain` minted only with translation on | On 2026-10-03 mutations 1,546, 1,550 and 1,554 each failed only their named check |
| PW-569 | DONE | P0 | No hardware boundary separated published from retired DMA buffers | D-358 maps each buffer before its doorbell and unmaps it with an invalidation when host-owned | On 2026-10-03 mutations 1,545 and 1,544 each failed only its machine check |
| PW-570 | DONE | P0 | Storage started even without a usable remapping unit | D-357, D-358 and D-388 refuse such candidates with `storage: no DMA isolation` | On 2026-10-03 `cargo xtask degraded-check --release` passed; mutations 1,549, 1,540 and 1,541 failed their checks |
| PW-571 | DONE | P0 | The VT-d path was proven only on QEMU's unit | The generated self-test and shipped machines ask for VMware's virtual VT-d unit, and `cargo xtask qualify-vmware` observes `iommu: translation on` before `nvme: up` in both fresh transcripts | Both fresh VMware qualifications of `694061f` record `iommu: translation on` before `nvme: up` (D-449, v0.2.0) |
| PW-572 | DONE | P0 | The debug candidate path reached the 32 KiB boot stack's limit | D-359 makes `try_controller` a thin sequencer of sibling stages | On 2026-10-03 every debug path kept 8,560 bytes free; mutation 1,570 reds the contract |
| PW-573 | DONE | P1 | Reserved DMA regions went unchecked and `DmaDomain` was copyable (Daybreak findings 1 and 8) | D-360 makes the domain linear; D-388 refuses every firmware-reserved DMA region and D-389 removes its argument | On 2026-10-11 mutation 1,589 reddened the source contract and both self-tests passed |
| PW-574 | DONE | P1 | Four source checks missed shapes they never read (Daybreak findings 5, 11, 13 and 15) | D-360 refuses imported raw doors, odd manifest headers and outside path packages | On 2026-10-03 mutations 1,581 through 1,588 each failed its named test |
| PW-575 | DONE | P1 | D-326's stack meter missed frames live between register accesses | D-361 paints the dead boot stack per candidate and refuses incoherent readings | On 2026-10-03 mutations 1,590 through 1,596 each failed their check |
| PW-576 | DONE | P1 | Manifest assignments and macros could hide dependencies and raw port claims | D-362 bounds manifest assignments and inspects macro tokens | On 2026-10-07 mutations 1,597, 1,598 and 1,602 each failed one exact test (D-362) |
| PW-577 | DONE | P2 | The handoff check pinned D-142 and D-148 to READY | D-363 derives those dispositions from their owners' current state | On 2026-10-07 mutations 1,599 through 1,601 each reddened the fixture (D-363) |
| PW-578 | DONE | P1 | VMware's VT-d width selector rejected an AGAW wider than MGAW | D-364 separates table depth from address coverage and enforces both bounds | Caught in the complete 2,412-entry sweep on `694061f`; VMware passed translation and storage (D-449, v0.2.0) |
| PW-579 | DONE | P1 | Qualification refused UART CRLF and Workstation's runtime VMX additions | D-365 validates terminators and constrains runtime additions; D-431 admits only a VNC group at `127.0.0.1:5997` | Caught in the complete 2,412-entry sweep on `694061f`; `qualify-vmware` accepted both fresh runs (D-449, v0.2.0) |
| PW-580 | DONE | P1 | VMware guests lack `RDCL_NO`, so shared-root ring 3 correctly refused | D-407 implements paired roots and requires exactly one selector-appropriate root record | Mutations 196 and 2,048 through 2,061 were caught in the complete 2,412-entry sweep on `694061f` (D-449, v0.2.0) |
| PW-581 | DONE | P0 | A malformed DMAR path or bridge overlap could misattribute a requester (Codex Security occurrences `occ_4ba847796226668b8378e266` and `occ_ecda4a10e95e394a518d2f7e`) | D-371 seals one complete PCI tree that every DMAR path must agree with | Mutations 1,680 through 1,702 each reddened their check; the 2026-10-09 gate passed |
| PW-582 | DONE | P0 | Caller-selected result directories could counterfeit passing records (Codex Security occurrence `occ_b06f0364edc18c50bc4df22d`) | D-373 removes both seal actions; D-406 adds the owned live Deep capability | Qualification v5 and evidence v9 invalidated the seal formats |
| PW-583 | DONE | P1 | The pending-IRR witness false-failed behind a higher timer request | D-374 tests the queued self-IPI's exact bit | Retained mutation 1,759 and the restored boot prove the witness |
| PW-584 | DONE | P0 | Local-APIC takeover left firmware LVT sources armed (Codex Security occurrence `occ_0569e6f2518e11797af06c04`) | D-375 masks and reads back every advertised LVT entry before publication | Mutations 1,760 through 1,765 each red their witness; release QEMU passes 279/279 |
| PW-585 | DONE | P0 | Safe mapping doors could create writable executable pages (Codex Security scan `32245094-b8b7-47d1-b6e7-a52554800772`) | D-376 refuses `WriteExecute` at the private `map_range` boundary before any change | Mutation 1,766 failed only `writable executable mappings leave no trace` |
| PW-586 | DONE | P1 | CI preflight read `README.md` unbounded through links (Codex Security occurrence `occ_33f31e610855178e51859b01`) | D-377 uses D-344's no-follow bounded reader with a 64 KiB ceiling | Mutation 1,767 fails exactly the link witness |
| PW-587 | DONE | P1 | The polled-reader witness could flake on correct behavior | D-378 waits for reader completion and a later poller pass together | Mutation 1,768 fails the source contract |
| PW-588 | DONE | P1 | Image routes read boot files unbounded through links (Codex Security occurrence `occ_a2d96237da4d5f8b84e5da46`) | D-379 reads them through D-344's no-follow boundary with fixed ceilings | Mutation 1,769 fails exactly the link witness |
| PW-589 | DONE | P1 | Direct image commands skipped `BOOTX64.EFI`'s pinned digest (Codex Security occurrence `occ_ef7f6498ac803835882433e5`) | D-380 authenticates the bounded buffer against its sole `CHECKSUMS` entry and writes that buffer | Mutation 1,770 fails exactly the substitution witness |
| PW-590 | DONE | P1 | Trap-side `OpenAt` could reach a lock that halts the kernel (Codex Security occurrence `occ_58f0b32a48b6c444cfe0d501`) | D-381 limits trap-side admission to reservation and queueing; the service worker alone resolves | Mutations 1,771 and 1,772 failed; the restored tree passed 281/281 |
| PW-591 | DONE | P0 | VT-d revocation did not drain in-flight DMA (Codex Security occurrence `occ_a1a268487e204fdeb7611fcf`) | D-383 flushes and drains reads and writes after every live revocation and keeps ownership on uncertain failure | Mutations 1,776 through 1,784 each failed their witness |
| PW-592 | DONE | P1 | Cargo-mutants command tests refused the nested sweep workspace's baseline | D-385 gives those fixtures an exclusive temporary repository | The complete 2,412-entry sweep on `694061f` passed its baseline (D-449, v0.2.0) |
| PW-593 | DONE | P0 | A mutable PCI snapshot could authorize VT-d takeover with translation off (Codex Security occurrence `occ_9a4891f61ef72658a0f921ef`) | D-389 sets one default-deny owner from static authority before PCI discovery; firmware remapping must already be inactive, and an uncertain owner is permanently quarantined | Mutations 1,803 through 1,821 each reddened their check |
| PW-594 | DONE | P0 | Early boot state was built before the first requester fence (Codex Security occurrence `occ_dcbc2eb04409f8aea9aba2df`) | D-390 fences every BDF before memory initialization on QEMU TCG and VMware only; D-392 exempts only VMware's 440BX, and every other platform refuses | Caught in the complete 2,412-entry sweep on `694061f`; VMware records the 440BX non-requester (D-449, v0.2.0) |
| PW-595 | DONE | P1 | The API audit's compiler output exceeded its 64 MiB capture ceiling | D-391 and D-393 stream and keep only bounded verdict evidence | Caught in the complete 2,412-entry sweep on `694061f`; `cargo xtask ci` passed (D-449, v0.2.0) |
| PW-596 | DONE | P0 | VMware's 440BX host bridge has a read-only command register | D-392 admits only that exact bridge as present but not a requester; QEMU stays strict | Mutations 1,840 through 1,865 were caught in the complete 2,412-entry sweep on `694061f` (D-449, v0.2.0) |
| PW-597 | DONE | P0 | VMware's PIIX4 ISA bridge cannot clear bus mastering and masters ISA DMA | D-426 quiesces and masks both 8237 controllers and keeps the bridge fenced as a requester; D-428 binds the port protocol | Caught in the complete 2,412-entry sweep on `694061f`; VMware consumes the PIIX4 quiesced-requester record (D-449, v0.2.0) |
| PW-598 | DONE | P0 | Function authority capacity 12 was below VMware's 42 requesters | D-426 sets `pci_config::FUNCTION_AUTHORITY_LIMIT = 64` for both layers | Caught in the complete 2,412-entry sweep on `694061f`; VMware records `translation on` (D-449, v0.2.0) |
| PW-599 | DONE | P0 | VMware faulted writing the accessed bit into the read-only KPTI GDT | D-426 writes every code and data descriptor already accessed | Caught in the complete 2,412-entry sweep on `694061f`; VMware passes the ring-three checks (D-449, v0.2.0) |
| PW-713 | DONE | P1 | A caller could seal a PCI snapshot after an ignored observation error | D-432 revokes authority before each observation and keeps the first error until reset | Caught in the complete 2,412-entry sweep on `694061f` (D-449, v0.2.0) |
| PW-714 | DONE | P0 | Safe destruction could recycle DMA data or tables hardware could still reach | D-433 permanently keeps published table owners and releases segment owners only on proven revocation | Caught in the complete 2,412-entry sweep on `694061f`; VMware passed (D-449, v0.2.0) |
| PW-715 | DONE | P1 | The reservation-cleanup fixture stopped at an earlier permission gate | D-434 binds the fixture to a duplicable source that reaches the intended gate | Caught in the complete 2,412-entry sweep on `694061f` (D-449, v0.2.0) |
| PW-716 | DONE | P1 | QEMU errors, polls, joins and QMP writes could outlive their deadlines | D-435 owns the child immediately and bounds every stop, join and write | Mutation 813 and its siblings were caught in the complete 2,412-entry sweep on `694061f` (D-449, v0.2.0) |
| PW-717 | DONE | P1 | Three mutation readers could follow a substituted final link | D-436 routes them through `open_real_file` | Caught in the complete isolated sweep on `694061f` (D-449, v0.2.0) |
| PW-718 | DONE | P1 | Known options were silently ignored on unrelated tasks | D-437 validates every option against its consuming task before dispatch | Caught in the complete 2,412-entry sweep on `694061f` (D-449, v0.2.0) |
| PW-719 | DONE | P1 | The delayed-read witness could block forever and keep its NVMe hold | D-438 gives that round a nonblocking finish that always releases the hold | Caught in the complete 2,412-entry sweep on `694061f`; VMware passed (D-449, v0.2.0) |
| PW-720 | DONE | P1 | Pollers could enqueue input out of order and count refused events | D-439 excludes interruption across each drain and advances only on admission | Caught in the complete 2,412-entry sweep on `694061f`; VMware passed (D-449, v0.2.0) |
| PW-721 | DONE | P1 | The clock-resolution witness measured loop cost rather than clock resolution | D-443 measures adjacent timestamp pairs through the public conversion helper | Mutations 2,441 and 2,442 were caught in release and debug boots of `694061f` (D-449, v0.2.0) |
| PW-722 | DONE | P1 | The 294-check kernel exceeded D-384's resource bands | D-445 recenters only those bands with unchanged widths | Mutation 534 was caught; `cargo xtask baseline` passed within D-445's bands (D-449, v0.2.0) |
| PW-723 | DONE | P1 | A self-test wait could spin forever after watchdog revocation froze time | D-446 rechecks calibration before each predicate and refuses after revocation | Mutation 2,444 produced its exact red in release and debug boots of `694061f` (D-449, v0.2.0) |

## Compiler gate and repository handoff

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| WZ-001 | WIZZY | GATE | The inspected compiler revision was local-only | Publish/reachably pin one `wizards-ink` revision, toolchain and every dependency | A clean machine can fetch exactly the recorded inputs; no sibling working tree is implied |
| WZ-002 | WIZZY | GATE | Wizzy has no non-interactive standalone program artifact | Add one documented command from standalone source to a versioned loadable artifact | Artifact identifies/rejects versions and runs without compiler, checked AST, compiler maps or host services |
| WZ-003 | WIZZY | GATE | The VM/runtime retains compiler structures and uses `std` host facilities | Separate the execution runtime behind a host-independent platform seam and build it with `no_std + alloc` | Target runtime dependency audit contains no compiler/AST/host I/O; host and minimal target adapter run the same fixture |
| WZ-004 | WIZZY | GATE | Reproducible compiler output is not established | Lock inputs and make two clean builds byte-identical | Gate record includes both commands, environments and matching artifact hashes |
| PW-600 | DONE | GATE | The shared handoff had no machine-readable version record | D-226 defines a route-neutral envelope and allocation-free parser | 2026-08-29 every mismatch was refused; mutations 504-511 persist |
| PW-601 | TRIGGER | WHEN-TRIGGERED | Consumer code before a real artifact would freeze guesses | Realm owns the loader/runner scaffold under D-366; Bedrock admits only kernel ABI work forced by that pinned WZ-002/WZ-003 caller | Trigger: a pinned draft artifact plus runtime seam; proof consumes committed/vendored bytes and compares regenerated digests separately |
| PW-602 | TRIGGER | WHEN-TRIGGERED | Opening from a developer checkout would make the gate non-reproducible | Realm owns the D-183 gate-opening decision after WZ-001 through WZ-004 pass together; Bedrock separately pins that decision and admits its kernel integration under D-366 | Trigger: all four rows at one revision; Realm records pins, commands, hashes and seam, while Bedrock records exact admitted inputs and keeps ordinary kernel CI sibling-independent |

## Trigger-only kernel/platform work

| ID | State | Priority | Why this is live | Deliverable | Proof required to close |
|---|---|---|---|---|---|
| PW-700 | TRIGGER | WHEN-TRIGGERED | Frame quotas and per-message allocation accounting have no current user-controlled allocation route | Add accounting at the admission owner selected by the actual runtime/channel caller | Trigger: first user spawn/map/channel-send route; over-quota refusal leaks nothing and cannot starve another process |
| PW-701 | TRIGGER | WHEN-TRIGGERED | Dynamic stacks and a first-class `MemoryObject` need measured runtime allocation/zero-copy behavior | Design them against the chosen Wizzy runtime rather than today's probe | Trigger: post-gate route measurement; repeated growth/shrink/exit is W^X, guarded and resource-steady |
| PW-702 | TRIGGER | WHEN-TRIGGERED | Usercopy fault fixup has no unmap race while a process has one thread | Stress/fix the race when another thread can revoke the copied range | Trigger: multi-threaded process or asynchronous unmap; no stale access, livelock or leaked recovery state |
| PW-703 | TRIGGER | WHEN-TRIGGERED | Stack-token middle-page validation matters only when stacks grow or shrink | Make stack evidence linear or validate its complete dynamic extent | Trigger: dynamic stack mutation; removing any page invalidates the token before CPU installation |
| PW-704 | TRIGGER | WHEN-TRIGGERED | Page cache, writeback and replacing bounded polled NVMe service with deferred or interrupt-driven service need a workload, not intuition; one known cost is a FAT `Volume::read_file` near the 4 GiB offset, which walks the file's chain from its first cluster with only the volume's cluster count as its bound and stays unreachable while the shipped kernel grants ring 3 no file capability | Measure the first real Wizzy storage workload and decide cache, writeback and service ownership together | Trigger: pinned Wizzy workload after route integration; throughput, latency and memory pressure justify the design |
| PW-705 | TRIGGER | WHEN-TRIGGERED | Toolchain, Limine/base-revision and host-only dependency workarounds, runtime logging controls and extra ABI integer/buffer types have no current forcing event | Re-evaluate a pin or workaround only when stable support appears, it blocks a supported host or required dependency, a configuration source exists, or a source-level Wizzy caller requires the ABI type | Trigger is named in the change; no dependency/API upgrade lands solely because it exists |
| PW-706 | DONE | P0 | The VMware target lacks `RDCL_NO`, so D-250 closes shared-root ring 3 and no host log can reopen it | D-407's paired-root KPTI path is implemented with selector-specific VMware verification | Caught in the complete 2,412-entry sweep on `694061f`; VMware proves the paired path (D-449, v0.2.0) |
| PW-707 | TRIGGER | WHEN-TRIGGERED | D-126 already replaced D-121's stale fourth-static threshold with an exercised lock-nesting witness; D-430 records D-200's existing atomic-only readiness callback without claiming whole-program callback enforcement | Before exposing new caller-supplied work or protected contents under a leaf guard, or expanding the audited readiness predicate, remove that exposure or add a bounded source/semantic contract for its exact authority | Trigger: that concrete callback-authority change, not a static count; a deliberate violation fails the exact contract. The current `Queue::wait_from_trap` predicate only reads the atomic completion generation, and the existing machine witness checks exercised lock nesting |
| PW-708 | TRIGGER | WHEN-TRIGGERED | D-441 permits Windows-only Source preview checks; the portability code still has no non-Windows execution proof | Restore a hosted supported-host gate when a second active contributor or serious non-Windows support makes that proof authoritative rather than duplicative; that host also runs bounded commands in process groups so each ends with its tree (D-347) | Trigger: either condition is recorded; the restored gate runs the same typed project stages without inventing a second source of truth |
| PW-709 | DONE | P0 | Release kernels embedded host paths that evidence from one checkout could not detect | D-386 remaps every host root to fixed prefixes and builds evidence from independent roots | `cargo xtask evidence` on `694061f` built byte-identical bytes free of host roots (D-449, v0.2.0) |
| PW-710 | DONE | P1 | A hostile PCI function could forge MSI vectors with no interrupt remapping (Codex Security occurrence `occ_66ce12f335b236e1daa2b291`) | D-372 installs a deny-all VT-d interrupt table before BME or `sti` and consumes a linear `InterruptIsolation` | Mutations 1,703 through 1,742 each reddened their check; firmware/SMM, devices and remapping-hardware event sources are assumed quiescent for compatibility-format interrupt traffic only until the first successful kernel IRE establishment, and firmware/SMM is trusted not to reprogram VT-d or APIC after the kernel's readbacks |
| PW-711 | TRIGGER | WHEN-TRIGGERED | The DMA-isolation unit is Intel VT-d; AMD's IOMMU is a different register and table encoding for the same reach guarantee, and no qualified host presents one today | Add an AMD-Vi backend behind the same DmaDomain admission seam only when a qualified AMD host becomes a boot target | Trigger: a qualified AMD host; proof programs the AMD device table and page tables, faults an unmapped access, and shares the fail-closed admission with the Intel path |
| PW-712 | TRIGGER | WHEN-TRIGGERED | The domain identity-maps live segments so IOVA equals physical; per-segment translation that also hides the physical map is stronger but not required for the reach guarantee, which already faults every access outside the live set | Add an IOVA allocator and non-identity second-level maps only when hiding the physical map from a device has a concrete caller | Trigger: a stated need to hide the physical map from a device; proof hands each segment a distinct IOVA, faults the physical alias, and keeps the fail-closed admission |

<!-- PRE-WIZZY-REGISTER:END -->

## Explicitly after the compiler gate

These are designed in `015-wizzy-userland.md`, but they are not pre-Wizzy
kernel work: ring-3 channel create/send/receive, handle close, a user-visible
Device handle, the bootstrap
message, terminal and directory delegation, enumeration, VM map/unmap and
runtime allocation, the Wizzy platform adapter and capability values, process
objects, spawn-from-image, exit status, `init`, TTY, shell, pipelines, and
tmpfs mutation. Their order begins with a measured real artifact; none may be
pulled forward by relabeling it "foundation".

`019-userland.md` designs the userland these items serve and lists further
post-gate prerequisites with their callers (D-314); it adds no row here.

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
| Deliberately absent | xAPIC fallback for out-of-scope old CPUs, kernel Wizzy runtime/compiler, Rust fallback userland, speculative multi-queue waiting or POSIX/native ABI growth without a concrete caller, full functional verification, and FRED implementation without a hardware/toolchain caller |

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
optional hardening. The VMware observation belongs to PW-301 and PW-003;
D-225 closes PW-303's clocksource qualification and post-boot revocation
without upgrading it.

PW-514 carried the stale current-facing corrections in `000-vision.md`,
`001-boot.md`, `009-multitasking.md`, `011-userspace.md` and
`012-research-notes-4.md`; D-219 closes all of them without upgrading
historical evidence. Limine remains the boot boundary, and an own loader needs
a concrete unmet handoff rather than a roadmap slot.

### Historical D-121 through D-156 handoff reconciliation

This is the current disposition of the adversarial handoff decisions. A
`TRIGGER` remains stopped, a `READY` row names live work in the register, and a
review limit states exactly what automation still cannot decide.

| Decision | Current disposition | Current evidence, owner, or trigger |
|---|---|---|
| D-121 | TRIGGER | D-126 supplies the exercised lock-nesting witness; D-430 records the existing atomic-only readiness exception. PW-707 fires on new caller-supplied work or protected-content exposure under a leaf guard, or expansion of that predicate. |
| D-122 | CLOSED | D-123 and the D-124 through D-156 audit sequence disposed its four findings. |
| D-123 | CLOSED | D-124 and D-136 through D-156 supplied the mutation, gate, ABI and prose follow-through. |
| D-124 | CLOSED / REVIEW LIMIT | Existence checking is mechanical; semantic agreement between a citation and its ADR still requires review. |
| D-125 | CLOSED | D-337 closes the live-root and complete retained-stack proof; PW-703 is only the later dynamic-stack trigger. |
| D-126 | CLOSED | D-154 supplied the per-CPU counter seam. |
| D-127 | SUPERSEDED | D-248 removed the speculative Event, peer, transport, wait and close surface. |
| D-128 | CLOSED / REVIEW LIMIT | D-182 replaced on-demand arena growth with pre-admission capacity; runtime detection still covers only walked paths. |
| D-129 | TRIGGER | The remaining two-caller PIC race waits for Phase 12 SMP; D-138 paid atomic pair acquisition. |
| D-130 | SUPERSEDED | D-131/D-132 corrected lint limits and D-248 removed the speculative dead-code surface. |
| D-131 | CLOSED | The remaining unsafe allowances are permissions, not claims that can be falsified by a mutation. |
| D-132 | CLOSED | D-213 replaced text reachability with stable-rustc semantic caller evidence. |
| D-133 | CLOSED | D-223 built the missing production RingEnter wake witness. |
| D-134 | CLOSED / REVIEW LIMIT | D-209 binds a green baseline and exact red verdict; causal intent beyond that transition remains review work. |
| D-135 | CLOSED | D-211 typed the image selector and D-240 bounded the human round; current execution evidence remains PW-003. |
| D-136 | CLOSED | D-328 closes the last two exact compiler reintroductions in PW-519 and PW-523; PW-212, PW-309, PW-518, PW-520, PW-521, PW-524 and PW-526 record the other resolved findings. |
| D-137 | CLOSED | D-138 supplied atomic multi-range acquisition; citation meaning remains D-124's review limit. |
| D-138 | CLOSED | Pairwise atomic claims exist; no current caller needs a larger-span generalization. |
| D-139 | CLOSED | D-140 repaired false safety reasoning and stale roots; D-214 closed retained-pointer lifetime review. |
| D-140 | CLOSED / REVIEW LIMIT | Safety prose is inventoried, but its truth still requires reading the implementation and caller. |
| D-141 | CLOSED | D-255 and PW-027 prove the fired condition through the indivisible production claim and its retained install-removal mutation. |
| D-142 | CLOSED | PW-128 and PW-201 closed with v0.2.0 (D-449). |
| D-143 | TRIGGER | Per-CPU/ring detector work waits for Phase 12 SMP. |
| D-144 | CLOSED | D-228 observes x87, SSE and AVX independently; PW-305 still verifies compacted XSAVEC hardware. |
| D-145 | SUPERSEDED | D-248 removed the receive transport; a future post-Wizzy protocol is a new design, not this debt. |
| D-146 | CLOSED | D-215 checks operational Markdown and current numeric authority. |
| D-147 | CLOSED | D-212 replaced handwritten CI transcription with one typed registry. |
| D-148 | CLOSED | PW-012 paid transcript retention; PW-002 and PW-009 closed with v0.2.0 (D-449). |
| D-149 | CLOSED | D-168 through D-173 supplied the device/filesystem decisions and D-181 repaired ABI status. |
| D-150 | CLOSED / REVIEW LIMIT | D-247 and PW-530 require explicit direct-door classification; whether a call can wait remains semantic review. |
| D-151 | CLOSED | D-163 made reservations destructor-backed and D-167 settled retired-program work. |
| D-152 | CLOSED | D-166 supplied the versioned time page and exact cross-check. |
| D-153 | CLOSED | D-164 added cancellation and D-167 quietens a retired ring before release. |
| D-154 | TRIGGER | The per-CPU seam exists; stress and flake policy wait for Phase 12 SMP. |
| D-155 | CLOSED | D-156 paid the final scancode-table item. |
| D-156 | CLOSED | D-162 independently anchored the set-2 backslash code. |

## What is left

v0.2.0 closed every row its gate proved (D-449); the order below is for the
next qualified release.

The failed `f28428a`, `44e1c89`, `0dfcb76` and `20cf580` reports are
historical repair inventories, not qualification; PW-001 records their counts.
The complete updated current registry must pass on the frozen qualification
commit; no earlier verdict may be inherited. D-425's qualified-release path,
stronger than D-441's source preview, remains in force. A future qualified
release needs another immutable version and the following finite order:

1. Close every source, proof and manual-red gap that does not depend on final
   source-bound qualification; draft PW-006's disposition ADR without inventing
   results, and keep machine-dependent rows open.
2. Freeze and commit a clean candidate. On that exact commit run CI, the full
   curated mutation sweep, model checks, the fresh advisory qualification, and
   both generated-and-human-booted VMware selectors. The `xtask` grade and the
   owned Deep scan run continuously against `main` and enter this release only
   when their records name this exact commit; otherwise the bundle declares
   each `deferred` (D-425). Both VMware transcripts must show `iommu: translation on`
    before `nvme: up` (PW-571). No verdict may be inherited from `ff6ddd3`,
    `e47c22c`, `a58bc97`, `f28428a` or `44e1c89`.
3. If any proof requires a source change, discard the candidate evidence and
   repeat. Once the candidate is green, create and independently verify
   evidence v11 on that same commit with D-330's `same-commit` relation, then
   tag it directly with its own new qualified-release version.
4. In the commit after the tag, record the exact results, close the register
   rows the gate proved and list every remaining live row as a known open item
   of that qualified release. That commit is not part of the release evidence.
   D-330's documentation-only child remains supported for a later close that needs it.
5. Stop at the handoff. WZ-001 through WZ-004 remain owned by `wizards-ink`.
   PW-601 stays stopped until its pinned draft-artifact/runtime-seam trigger;
   PW-602 and `015`'s product slices stay stopped until the gate-opening ADR.

This ordering is a risk order, not authorization to skip the register's proof
rules. Every implementation still gets its check, every new check is broken
once, and every machine check gets a persistent mutation.
