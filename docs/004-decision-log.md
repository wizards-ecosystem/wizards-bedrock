# 004 - Decision log

Each decision is one file in `decisions/`. A status is Accepted, Verified (backed by research
verified in `003`) or Provisional (built on, with a falsification condition, D-71). Decisions up
to D-20 were made on 2026-07-18.

| Decision | Status | Date |
|---|---|---|
| [D-01 Kernel language: Rust](decisions/0001-kernel-language-rust.md) | Verified | 2026-07-18 |
| [D-02 Target architecture: x86_64 first, ports kept possible](decisions/0002-target-architecture-x86-64-first-ports-kept-possible.md) | Accepted | 2026-07-18 |
| [D-03 Boot: UEFI-only via vendored Limine](decisions/0003-boot-uefi-only-via-vendored-limine.md) | Accepted | 2026-07-18 |
| [D-04 Kernel shape: monolithic-modular](decisions/0004-kernel-shape-monolithic-modular.md) | Verified | 2026-07-18 |
| [D-05 Destination: full OS with userspace, browser, and Python](decisions/0005-destination-full-os-with-userspace-browser-and-python.md) | Accepted | 2026-07-18 |
| [D-06 Native ABI: capability handles + async submission/completion queues](decisions/0006-native-abi-capability-handles-async-submission-completion-queues.md) | Verified | 2026-07-18 |
| [D-07 Process model: spawn-only, no native fork](decisions/0007-process-model-spawn-only-no-native-fork.md) | Accepted | 2026-07-18 |
| [D-08 Filesystem: custom WizFS, CoW from the start](decisions/0008-filesystem-custom-wizfs-cow-from-the-start.md) | Provisional | 2026-07-18 |
| [D-09 App formats: ELF64 + WASI 0.3 components as first-class](decisions/0009-app-formats-elf64-wasi-0-3-components-as-first-class.md) | Provisional | 2026-07-18 |
| [D-10 POSIX as a translation layer only; mlibc is the ports gateway](decisions/0010-posix-as-a-translation-layer-only-mlibc-is-the-ports-gateway.md) | Accepted | 2026-07-18 |
| [D-11 Graphics: own compositor, Wayland-style protocol, GPU-first toolkit](decisions/0011-graphics-own-compositor-wayland-style-protocol-gpu-first-toolkit.md) | Accepted | 2026-07-18 |
| [D-12 Security baseline from day one](decisions/0012-security-baseline-from-day-one.md) | Accepted | 2026-07-18 |
| [D-13 Framekernel discipline: all `unsafe` in one privileged crate](decisions/0013-framekernel-discipline-all-unsafe-in-one-privileged-crate.md) | Provisional | 2026-07-18 |
| [D-14 Fault containment by supervision, not unwinding](decisions/0014-fault-containment-by-supervision-not-unwinding.md) | Verified | 2026-07-18 |
| [D-15 Scheduler: EEVDF-inspired virtual deadlines, tickless](decisions/0015-scheduler-eevdf-inspired-virtual-deadlines-tickless.md) | Accepted | 2026-07-18 |
| [D-16 Fully self-contained native-Windows build](decisions/0016-fully-self-contained-native-windows-build.md) | Accepted | 2026-07-18 |
| [D-17 Toolchain pins and workarounds](decisions/0017-toolchain-pins-and-workarounds.md) | Accepted | 2026-07-18 |
| [D-18 Test targets: QEMU is the loop, VMware is the milestone](decisions/0018-test-targets-qemu-is-the-loop-vmware-is-the-milestone.md) | Accepted | 2026-07-18 |
| [D-19 Method and voice](decisions/0019-method-and-voice.md) | Accepted | 2026-07-18 |
| [D-20 Networking plan: smoltcp in-kernel, rustls in userspace](decisions/0020-networking-plan-smoltcp-in-kernel-rustls-in-userspace.md) | Accepted | 2026-07-18 |
| [D-21 The framekernel crate split lands now, not later](decisions/0021-the-framekernel-crate-split-lands-now-not-later.md) | Accepted | 2026-08-12 |
| [D-22 Lint, test, and CI policy](decisions/0022-lint-test-and-ci-policy.md) | Accepted | 2026-08-12 |
| [D-23 xtask is a library with a thin binary](decisions/0023-xtask-is-a-library-with-a-thin-binary.md) | Accepted | 2026-08-12 |
| [D-24 The toolchain pin is exact](decisions/0024-the-toolchain-pin-is-exact.md) | Accepted | 2026-08-12 |
| [D-25 Vendored binaries are checksum-pinned](decisions/0025-vendored-binaries-are-checksum-pinned.md) | Accepted | 2026-08-12 |
| [D-26 No hosted CI; the local gate is the gate](decisions/0026-no-hosted-ci-the-local-gate-is-the-gate.md) | Accepted | 2026-08-12 |
| [D-27 The Rust API Guidelines are the house style](decisions/0027-the-rust-api-guidelines-are-the-house-style.md) | Accepted | 2026-08-12 |
| [D-28 On-machine tests via QEMU's exit device](decisions/0028-on-machine-tests-via-qemus-exit-device.md) | Accepted | 2026-08-12 |
| [D-29 Interrupt entry is generated assembly, not a nightly ABI](decisions/0029-interrupt-entry-is-generated-assembly-not-a-nightly-abi.md) | Accepted | 2026-08-13 |
| [D-30 Descriptor encoding is a host-tested library](decisions/0030-descriptor-encoding-is-a-host-tested-library.md) | Accepted | 2026-08-13 |
| [D-31 A fault is terminal and reported; a breakpoint resumes](decisions/0031-a-fault-is-terminal-and-reported-a-breakpoint-resumes.md) | Accepted | 2026-08-13 |
| [D-32 The CPU protection baseline goes on at boot](decisions/0032-the-cpu-protection-baseline-goes-on-at-boot.md) | Accepted | 2026-08-13 |
| [D-33 x2APIC only, because xAPIC needs page tables](decisions/0033-x2apic-only-because-xapic-needs-page-tables.md) | Accepted | 2026-08-14 |
| [D-34 A spin lock holds the interrupt flag](decisions/0034-a-spin-lock-holds-the-interrupt-flag.md) | Accepted | 2026-08-14 |
| [D-35 Enabling interrupts is safe because both preconditions are checked](decisions/0035-enabling-interrupts-is-safe-because-both-preconditions-are-checked.md) | Accepted | 2026-08-14 |
| [D-36 The timer is calibrated against the 8254, and ticks at 100 Hz](decisions/0036-the-timer-is-calibrated-against-the-8254-and-ticks-at-100-hz.md) | Accepted | 2026-08-20 |
| [D-37 Keyboard input is polled until the I/O APIC exists](decisions/0037-keyboard-input-is-polled-until-the-i-o-apic-exists.md) | Accepted | 2026-08-20 |
| [D-38 The harness can press a key](decisions/0038-the-harness-can-press-a-key.md) | Accepted | 2026-08-20 |
| [D-39 A port claim covers the width it drives](decisions/0039-a-port-claim-covers-the-width-it-drives.md) | Accepted | 2026-08-22 |
| [D-40 The linked image is verified, not assumed](decisions/0040-the-linked-image-is-verified-not-assumed.md) | Accepted | 2026-08-22 |
| [D-41 Three authorities for `unsafe`, and doctests where they can run](decisions/0041-three-authorities-for-unsafe-and-doctests-where-they-can-run.md) | Accepted | 2026-08-22 |
| [D-42 Limine base revision 4](decisions/0042-limine-base-revision-4.md) | Accepted | 2026-08-22 |
| [D-43 The frame allocator: mechanism in `wost`, policy in `libs/`](decisions/0043-the-frame-allocator-mechanism-in-wost-policy-in-libs.md) | Accepted | 2026-08-22 |
| [D-44 Untyped memory is a type discipline, not an accounting mechanism](decisions/0044-untyped-memory-is-a-type-discipline-not-an-accounting-mechanism.md) | Accepted | 2026-08-22 |
| [D-45 The kernel's own tables, and a `CR3` switch that is a safe function](decisions/0045-the-kernels-own-tables-and-a-cr3-switch-that-is-a-safe-function.md) | Accepted | 2026-08-22 |
| [D-46 KASLR waits, and this is what waiting costs](decisions/0046-kaslr-waits-and-this-is-what-waiting-costs.md) | Accepted | 2026-08-22 |
| [D-47 The kernel heap lives in the direct map](decisions/0047-the-kernel-heap-lives-in-the-direct-map.md) | Accepted | 2026-08-22 |
| [D-48 The console scrolls out of RAM](decisions/0048-the-console-scrolls-out-of-ram.md) | Accepted | 2026-08-22 |
| [D-49 Devices are mapped before the switch, and the keyboard interrupts](decisions/0049-devices-are-mapped-before-the-switch-and-the-keyboard-interrupts.md) | Accepted | 2026-08-22 |
| [D-50 The framebuffer is write-combining](decisions/0050-the-framebuffer-is-write-combining.md) | Accepted | 2026-08-22 |
| [D-51 The context switch is a naked function, and a switch is a safe call](decisions/0051-the-context-switch-is-a-naked-function-and-a-switch-is-a-safe-call.md) | Accepted | 2026-08-22 |
| [D-52 Thread stacks come from a boot-time arena, because mapping is one-way](decisions/0052-thread-stacks-come-from-a-boot-time-arena-because-mapping-is-one-way.md) | Accepted | 2026-08-22 |
| [D-53 Scheduling policy is arithmetic in `libs/eevdf`, behind a seam with a fallback](decisions/0053-scheduling-policy-is-arithmetic-in-libs-eevdf-behind-a-seam-with-a-fallback.md) | Accepted | 2026-08-22 |
| [D-54 Preemption: cooperative first, then the timer at trap exit, with "was interruptible" as the test](decisions/0054-preemption-cooperative-first-then-the-timer-at-trap-exit-with-was-interruptible.md) | Accepted | 2026-08-22 |
| [D-55 Trap context is a count the heap and the scheduler can ask](decisions/0055-trap-context-is-a-count-the-heap-and-the-scheduler-can-ask.md) | Accepted | 2026-08-22 |
| [D-56 Per-CPU state is one word, shaped to widen](decisions/0056-per-cpu-state-is-one-word-shaped-to-widen.md) | Accepted | 2026-08-22 |
| [D-57 The tick is retired: the counter is the clock, the timer is an alarm](decisions/0057-the-tick-is-retired-the-counter-is-the-clock-the-timer-is-an-alarm.md) | Accepted | 2026-08-23 |
| [D-58 Sleeping: a deadline, a preserved lag, and a halt for the degenerate case](decisions/0058-sleeping-a-deadline-a-preserved-lag-and-a-halt-for-the-degenerate-case.md) | Accepted | 2026-08-23 |
| [D-59 A thread's first interrupt flag comes from its creator](decisions/0059-a-threads-first-interrupt-flag-comes-from-its-creator.md) | Accepted | 2026-08-23 |
| [D-60 The idle thread is outside the policy, not at the bottom of it](decisions/0060-the-idle-thread-is-outside-the-policy-not-at-the-bottom-of-it.md) | Accepted | 2026-08-23 |
| [D-61 Waiting on an event: a bitmask queue, and a reschedule at trap exit](decisions/0061-waiting-on-an-event-a-bitmask-queue-and-a-reschedule-at-trap-exit.md) | Accepted | 2026-08-23 |
| [D-62 The context token arrives with the API that needed it](decisions/0062-the-context-token-arrives-with-the-api-that-needed-it.md) | Accepted | 2026-08-23 |
| [D-63 The console keeps interrupts on, and a lock kind is a rule](decisions/0063-the-console-keeps-interrupts-on-and-a-lock-kind-is-a-rule.md) | Accepted | 2026-08-23 |
| [D-64 A physical read is bounded by the memory map, not by a comment](decisions/0064-a-physical-read-is-bounded-by-the-memory-map-not-by-a-comment.md) | Accepted | 2026-08-23 |
| [D-65 The gate boots three kernels](decisions/0065-the-gate-boots-three-kernels.md) | Accepted | 2026-08-23 |
| [D-66 Setting the interrupt flag with a spin lock held is a checked bug](decisions/0066-setting-the-interrupt-flag-with-a-spin-lock-held-is-a-checked-bug.md) | Accepted | 2026-08-23 |
| [D-67 The `unsafe_code` default is deny, and `wost` lifts it in one visible place](decisions/0067-the-unsafe-code-default-is-deny-and-wost-lifts-it-in-one-visible-place.md) | Accepted | 2026-08-23 |
| [D-68 A lock guard cannot leave the CPU that took it](decisions/0068-a-lock-guard-cannot-leave-the-cpu-that-took-it.md) | Accepted | 2026-08-23 |
| [D-69 The lock discipline belongs in the borrow checker](decisions/0069-the-lock-discipline-belongs-in-the-borrow-checker.md) | Accepted | 2026-08-23 |
| [D-70 SMP: the target model, and the rules that cost nothing now](decisions/0070-smp-the-target-model-and-the-rules-that-cost-nothing-now.md) | Accepted | 2026-08-23 |
| [D-71 An ADR may be Provisional, and a Provisional ADR names what would refute it](decisions/0071-an-adr-may-be-provisional-and-a-provisional-adr-names-what-would-refute-it.md) | Accepted | 2026-08-23 |
| [D-72 WASI: `wasip1` is the target, and the Component Model is a watch item](decisions/0072-wasi-wasip1-is-the-target-and-the-component-model-is-a-watch-item.md) | Provisional | 2026-08-23 |
| [D-73 The filesystem is a destination, not a phase](decisions/0073-the-filesystem-is-a-destination-not-a-phase.md) | Provisional | 2026-08-23 |
| [D-74 mlibc arrives with the ABI, not after the userland](decisions/0074-mlibc-arrives-with-the-abi-not-after-the-userland.md) | Accepted | 2026-08-23 |
| [D-75 Phase 11 is an integration milestone, not a browser](decisions/0075-phase-11-is-an-integration-milestone-not-a-browser.md) | Accepted | 2026-08-23 |
| [D-76 The unsafe boundary holds against every safe caller, not just this kernel](decisions/0076-the-unsafe-boundary-holds-against-every-safe-caller-not-just-this-kernel.md) | Accepted | 2026-08-23 |
| [D-77 The completed-phase audit treats green as a claim to attack](decisions/0077-the-completed-phase-audit-treats-green-as-a-claim-to-attack.md) | Accepted | 2026-08-24 |
| [D-78 The handle table is one arena, and a handle is arithmetic](decisions/0078-the-handle-table-is-one-arena-and-a-handle-is-arithmetic.md) | Accepted | 2026-08-24 |
| [D-79 The queue is one mechanism, and overflow is impossible by construction](decisions/0079-the-queue-is-one-mechanism-and-overflow-is-impossible-by-construction.md) | Accepted | 2026-08-24 |
| [D-80 A signal shares the completion plane](decisions/0080-a-signal-shares-the-completion-plane.md) | Accepted | 2026-08-24 |
| [D-81 The trap entry is FRED-shaped, and FRED is not implemented](decisions/0081-the-trap-entry-is-fred-shaped-and-fred-is-not-implemented.md) | Accepted | 2026-08-24 |
| [D-82 Extended state is eager, compacted, and sized at boot](decisions/0082-extended-state-is-eager-compacted-and-sized-at-boot.md) | Accepted | 2026-08-24 |
| [D-83 Two rules that cost nothing and keep three options open](decisions/0083-two-rules-that-cost-nothing-and-keep-three-options-open.md) | Accepted | 2026-08-24 |
| [D-84 The ring protocol is modelled before it is written](decisions/0084-the-ring-protocol-is-modelled-before-it-is-written.md) | Accepted | 2026-08-24 |
| [D-85 A lock has two levels, and the leaf is the one a `Drop` can take](decisions/0085-a-lock-has-two-levels-and-the-leaf-is-the-one-a-drop-can-take.md) | Accepted | 2026-08-24 |
| [D-86 Mapping stops being one-way, because the refusal that made it safe is not the signature](decisions/0086-mapping-stops-being-one-way-because-the-refusal-that-made-it-safe-is-not-the.md) | Accepted | 2026-08-24 |
| [D-87 A mapping carries its evidence: owned frames, or a range the handoff described](decisions/0087-a-mapping-carries-its-evidence-owned-frames-or-a-range-the-handoff-described.md) | Accepted | 2026-08-24 |
| [D-88 Unmapping returns a token that owns the frames until it has flushed](decisions/0088-unmapping-returns-a-token-that-owns-the-frames-until-it-has-flushed.md) | Accepted | 2026-08-24 |
| [D-89 The double-fault handler gets a guarded stack; the boot thread's waits, with its cost](decisions/0089-the-double-fault-handler-gets-a-guarded-stack-the-boot-threads-waits-with-its.md) | Accepted | 2026-08-24 |
| [D-90 The gate boots a machine the kernel cannot fully use](decisions/0090-the-gate-boots-a-machine-the-kernel-cannot-fully-use.md) | Accepted | 2026-08-24 |
| [D-91 The speculation posture is read, reported, and decided in a `libs/` crate](decisions/0091-the-speculation-posture-is-read-reported-and-decided-in-a-libs-crate.md) | Accepted | 2026-08-24 |
| [D-92 The boot thread leaves the bootloader's stack, because that is the only way to guard it](decisions/0092-the-boot-thread-leaves-the-bootloaders-stack-because-that-is-the-only-way-to.md) | Accepted | 2026-08-24 |
| [D-93 Waking a thread from thread context owes it a scheduling decision](decisions/0093-waking-a-thread-from-thread-context-owes-it-a-scheduling-decision.md) | Accepted | 2026-08-24 |
| [D-94 The mid-phase audit: a refused mapping freed frames it had mapped](decisions/0094-the-mid-phase-audit-a-refused-mapping-freed-frames-it-had-mapped.md) | Accepted | 2026-08-24 |
| [D-95 The second audit: a fix that introduced undefined behaviour, and a wait that could sleep for ever](decisions/0095-the-second-audit-a-fix-that-introduced-undefined-behaviour-and-a-wait-that-could.md) | Accepted | 2026-08-24 |
| [D-96 Ring 3 arrives through a gate of its own, and `enter_user` checks rather than trusts](decisions/0096-ring-3-arrives-through-a-gate-of-its-own-and-enter-user-checks-rather-than.md) | Accepted | 2026-08-24 |
| [D-97 The user copy is the boundary, and its window is three instructions wide](decisions/0097-the-user-copy-is-the-boundary-and-its-window-is-three-instructions-wide.md) | Accepted | 2026-08-24 |
| [D-98 The return to ring 3 clears the buffers, and is not gated on a bit this machine cannot read](decisions/0098-the-return-to-ring-3-clears-the-buffers-and-is-not-gated-on-a-bit-this-machine.md) | Accepted | 2026-08-24 |
| [D-99 The ring-0 stack moves with the thread](decisions/0099-the-ring-0-stack-moves-with-the-thread.md) | Accepted | 2026-08-24 |
| [D-100 The program comes off an initrd, and the loader refuses what it may not map](decisions/0100-the-program-comes-off-an-initrd-and-the-loader-refuses-what-it-may-not-map.md) | Accepted | 2026-08-24 |
| [D-101 The third audit: ring 3 could switch SMAP off, and the flush was never executed](decisions/0101-the-third-audit-ring-3-could-switch-smap-off-and-the-flush-was-never-executed.md) | Accepted | 2026-08-24 |
| [D-102 A fault the program caused kills the program](decisions/0102-a-fault-the-program-caused-kills-the-program.md) | Accepted | 2026-08-24 |
| [D-103 A walk reports the permission the CPU will grant](decisions/0103-a-walk-reports-the-permission-the-cpu-will-grant.md) | Accepted | 2026-08-24 |
| [D-104 A second address space shares the kernel by reference and nothing else](decisions/0104-a-second-address-space-shares-the-kernel-by-reference-and-nothing-else.md) | Accepted | 2026-08-24 |
| [D-105 The fourth adversarial audit, over the kernel and Phases 0 to 6](decisions/0105-the-fourth-adversarial-audit-over-the-kernel-and-phases-06.md) | Accepted | 2026-08-24 |
| [D-106 A thread's address space is installed by the switch, and the binding is evidence](decisions/0106-a-threads-address-space-is-installed-by-the-switch-and-the-binding-is-evidence.md) | Accepted | 2026-08-24 |
| [D-107 The untyped split is a type, and "untyped" means externally modifiable](decisions/0107-the-untyped-split-is-a-type-and-untyped-means-externally-modifiable.md) | Accepted | 2026-08-24 |
| [D-108 Extended state is saved eagerly, by the same code that moves `CR3` and `RSP0`](decisions/0108-extended-state-is-saved-eagerly-by-the-same-code-that-moves-cr3-and-rsp0.md) | Accepted | 2026-08-24 |
| [D-109 The thread table and the run queue are separate structures](decisions/0109-the-thread-table-and-the-run-queue-are-separate-structures.md) | Accepted | 2026-08-24 |
| [D-110 Channels carry bytes and handles, and both ends declare the rights](decisions/0110-channels-carry-bytes-and-handles-and-both-ends-declare-the-rights.md) | Accepted | 2026-08-24 |
| [D-111 A program receives exactly one handle at birth, and it is a channel](decisions/0111-a-program-receives-exactly-one-handle-at-birth-and-it-is-a-channel.md) | Accepted | 2026-08-24 |
| [D-112 The queue protocol is checked before it is written, and each tier is asked only what it can answer](decisions/0112-the-queue-protocol-is-checked-before-it-is-written-and-each-tier-is-asked-only.md) | Accepted | 2026-08-24 |
| [D-113 The ring's pages are the kernel's, and every read of them happens once](decisions/0113-the-rings-pages-are-the-kernels-and-every-read-of-them-happens-once.md) | Accepted | 2026-08-24 |
| [D-114 The POSIX layer is written and not compiled, and the finding is what it was for](decisions/0114-the-posix-layer-is-written-and-not-compiled-and-the-finding-is-what-it-was-for.md) | Accepted | 2026-08-24 |
| [D-115 The Phase 5 closing audit](decisions/0115-the-phase-5-closing-audit.md) | Accepted | 2026-08-24 |
| [D-116 A program drives its own ring, and the shipped kernel runs one](decisions/0116-a-program-drives-its-own-ring-and-the-shipped-kernel-runs-one.md) | Accepted | 2026-08-24 |
| [D-117 A dead program's memory comes back, collected by whoever has nothing better to do](decisions/0117-a-dead-programs-memory-comes-back-collected-by-whoever-has-nothing-better-to-do.md) | Accepted | 2026-08-24 |
| [D-118 Thread-local storage is the fourth thing that follows a thread](decisions/0118-thread-local-storage-is-the-fourth-thing-that-follows-a-thread.md) | Accepted | 2026-08-24 |
| [D-119 Breaking the checks is a task, not a habit](decisions/0119-breaking-the-checks-is-a-task-not-a-habit.md) | Accepted | 2026-08-24 |
| [D-120 A fault in the user copy resumes, because the program is the untrusted party](decisions/0120-a-fault-in-the-user-copy-resumes-because-the-program-is-the-untrusted-party.md) | Accepted | 2026-08-24 |
| [D-121 A lock that forbids calling has no closure form](decisions/0121-a-lock-that-forbids-calling-has-no-closure-form.md) | Accepted | 2026-08-24 |
| [D-122 The syscall handler leaves a number; something else says it](decisions/0122-the-syscall-handler-leaves-a-number-something-else-says-it.md) | Accepted | 2026-08-24 |
| [D-123 Four ways safe code reached undefined behaviour, and the road back that was not the only one](decisions/0123-four-ways-safe-code-reached-undefined-behaviour-and-the-road-back-that-was-not.md) | Accepted | 2026-08-24 |
| [D-124 A constant that only equals itself, and a check that only measures its own description](decisions/0124-a-constant-that-only-equals-itself-and-a-check-that-only-measures-its-own.md) | Accepted | 2026-08-24 |
| [D-125 Evidence has a date, and a root match is not a re-walk](decisions/0125-evidence-has-a-date-and-a-root-match-is-not-a-re-walk.md) | Accepted | 2026-08-24 |
| [D-126 A leaf lock is the last lock, and now something says so](decisions/0126-a-leaf-lock-is-the-last-lock-and-now-something-says-so.md) | Accepted | 2026-08-24 |
| [D-127 A channel end that dies says so](decisions/0127-a-channel-end-that-dies-says-so.md) | Accepted | 2026-08-25 |
| [D-128 Growing the arena is two allocator operations, and one was left behind](decisions/0128-growing-the-arena-is-two-allocator-operations-and-one-was-left-behind.md) | Accepted | 2026-08-25 |
| [D-129 Three claims about a boot-once function, and the safe door nobody could use](decisions/0129-three-claims-about-a-boot-once-function-and-the-safe-door-nobody-could-use.md) | Accepted | 2026-08-25 |
| [D-130 An `allow` is unfalsifiable and an `expect` is not](decisions/0130-an-allow-is-unfalsifiable-and-an-expect-is-not.md) | Accepted | 2026-08-25 |
| [D-131 Where `expect` cannot reach, and the difference between a permission and a claim](decisions/0131-where-expect-cannot-reach-and-the-difference-between-a-permission-and-a-claim.md) | Accepted | 2026-08-25 |
| [D-132 A public function nothing calls is a check nobody wrote](decisions/0132-a-public-function-nothing-calls-is-a-check-nobody-wrote.md) | Accepted | 2026-08-25 |
| [D-133 A premise about every trap arm, kept in a different file from the arms](decisions/0133-a-premise-about-every-trap-arm-kept-in-a-different-file-from-the-arms.md) | Accepted | 2026-08-25 |
| [D-134 A mutation is caught when *its* check goes red, not when the boot does](decisions/0134-a-mutation-is-caught-when-its-check-goes-red-not-when-the-boot-does.md) | Accepted | 2026-08-25 |
| [D-135 The boot that was every phase's definition of done checked nothing](decisions/0135-the-boot-that-was-every-phases-definition-of-done-checked-nothing.md) | Accepted | 2026-08-25 |
| [D-136 An accessor nothing reads back, and a mapping that survives being permuted](decisions/0136-an-accessor-nothing-reads-back-and-a-mapping-that-survives-being-permuted.md) | Accepted | 2026-08-25 |
| [D-137 A stray cursor on every scroll, and three claims written the same day they were false](decisions/0137-a-stray-cursor-on-every-scroll-and-three-claims-written-the-same-day-they-were.md) | Accepted | 2026-08-25 |
| [D-138 A claim that cannot be released must be taken all at once](decisions/0138-a-claim-that-cannot-be-released-must-be-taken-all-at-once.md) | Accepted | 2026-08-25 |
| [D-139 The IDT is not the only door, and a guard nothing could observe](decisions/0139-the-idt-is-not-the-only-door-and-a-guard-nothing-could-observe.md) | Accepted | 2026-08-25 |
| [D-140 A `SAFETY` comment that credited a check nobody performed](decisions/0140-a-safety-comment-that-credited-a-check-nobody-performed.md) | Accepted | 2026-08-25 |
| [D-141 A slot handed out and not marked](decisions/0141-a-slot-handed-out-and-not-marked.md) | Accepted | 2026-08-25 |
| [D-142 Hardware constants checked against the manual, not against themselves](decisions/0142-hardware-constants-checked-against-the-manual-not-against-themselves.md) | Accepted | 2026-08-25 |
| [D-143 A tally beside an instruction, and a detector read too early](decisions/0143-a-tally-beside-an-instruction-and-a-detector-read-too-early.md) | Accepted | 2026-08-25 |
| [D-144 The save area was sized for one form and written by another's dispatch](decisions/0144-the-save-area-was-sized-for-one-form-and-written-by-anothers-dispatch.md) | Accepted | 2026-08-25 |
| [D-145 Three claims the code had outrun, and one of them lost messages](decisions/0145-three-claims-the-code-had-outrun-and-one-of-them-lost-messages.md) | Accepted | 2026-08-25 |
| [D-146 Numbers in prose, checked against the code they describe](decisions/0146-numbers-in-prose-checked-against-the-code-they-describe.md) | Accepted | 2026-08-25 |
| [D-147 Four holes in the things that check everything else](decisions/0147-four-holes-in-the-things-that-check-everything-else.md) | Accepted | 2026-08-25 |
| [D-148 The model could check nothing and report success](decisions/0148-the-model-could-check-nothing-and-report-success.md) | Accepted | 2026-08-25 |
| [D-149 The ABI is extended before the filesystem is written](decisions/0149-the-abi-is-extended-before-the-filesystem-is-written.md) | Accepted | 2026-08-25 |
| [D-150 No blocking-only door, as a project rule](decisions/0150-no-blocking-only-door-as-a-project-rule.md) | Accepted | 2026-08-25 |
| [D-151 A completion may carry a handle, and the receiver reserves its slot](decisions/0151-a-completion-may-carry-a-handle-and-the-receiver-reserves-its-slot.md) | Accepted | 2026-08-25 |
| [D-152 Time is read from a page, and the page is global](decisions/0152-time-is-read-from-a-page-and-the-page-is-global.md) | Accepted | 2026-08-25 |
| [D-153 An operation in flight is collected with its program's zombie](decisions/0153-an-operation-in-flight-is-collected-with-its-programs-zombie.md) | Accepted | 2026-08-25 |
| [D-154 SMP stays at Phase 12; its seam is paid now](decisions/0154-smp-stays-at-phase-12-its-seam-is-paid-now.md) | Accepted | 2026-08-25 |
| [D-155 Two checks whose inputs made the property invisible](decisions/0155-two-checks-whose-inputs-made-the-property-invisible.md) | Accepted | 2026-08-25 |
| [D-156 The scancode table is transcribed, and one code is deliberately unanchored](decisions/0156-the-scancode-table-is-transcribed-and-one-code-is-deliberately-unanchored.md) | Accepted | 2026-08-25 |
| [D-157 A check that quantifies must read its domain, not restate it](decisions/0157-a-check-that-quantifies-must-read-its-domain-not-restate-it.md) | Accepted | 2026-08-25 |
| [D-158 One ELF reader, and the verifier uses the loader's](decisions/0158-one-elf-reader-and-the-verifier-uses-the-loaders.md) | Accepted | 2026-08-25 |
| [D-159 The lints that were measured and not adopted](decisions/0159-the-lints-that-were-measured-and-not-adopted.md) | Accepted | 2026-08-25 |
| [D-160 A page is 4096 bytes in five places, and now something checks that](decisions/0160-a-page-is-4096-bytes-in-five-places-and-now-something-checks-that.md) | Accepted | 2026-08-25 |
| [D-161 The list a test sweeps is itself a list that can fall behind](decisions/0161-the-list-a-test-sweeps-is-itself-a-list-that-can-fall-behind.md) | Accepted | 2026-08-25 |
| [D-162 The last unanchored scancode was a misread column](decisions/0162-the-last-unanchored-scancode-was-a-misread-column.md) | Accepted | 2026-08-25 |
| [D-163 The handle table works through a shared reference, and a promise is a value](decisions/0163-the-handle-table-works-through-a-shared-reference-and-a-promise-is-a-value.md) | Accepted | 2026-08-25 |
| [D-164 A wait that is a wait](decisions/0164-a-wait-that-is-a-wait.md) | Accepted | 2026-08-25 |
| [D-165 A check that could not fail, found by breaking it](decisions/0165-a-check-that-could-not-fail-found-by-breaking-it.md) | Accepted | 2026-08-25 |
| [D-166 The time is a page, not a call](decisions/0166-the-time-is-a-page-not-a-call.md) | Accepted | 2026-08-25 |
| [D-167 A dead program's work is finished, not forgotten](decisions/0167-a-dead-programs-work-is-finished-not-forgotten.md) | Accepted | 2026-08-25 |
| [D-168 A driver claims a device, not a port range](decisions/0168-a-driver-claims-a-device-not-a-port-range.md) | Accepted | 2026-08-25 |
| [D-169 An NVMe controller, polled, and a disk with something on it](decisions/0169-an-nvme-controller-polled-and-a-disk-with-something-on-it.md) | Accepted | 2026-08-25 |
| [D-170 FAT32, an oracle, and a block layer that is one trait](decisions/0170-fat32-an-oracle-and-a-block-layer-that-is-one-trait.md) | Accepted | 2026-08-25 |
| [D-171 A VFS over directories, and a resolver with no way up](decisions/0171-a-vfs-over-directories-and-a-resolver-with-no-way-up.md) | Accepted | 2026-08-25 |
| [D-172 A second backend, and a device that is not a file](decisions/0172-a-second-backend-and-a-device-that-is-not-a-file.md) | Accepted | 2026-08-25 |
| [D-173 The file handle, and a buffer with no pointer in it](decisions/0173-the-file-handle-and-a-buffer-with-no-pointer-in-it.md) | Accepted | 2026-08-25 |
| [D-174 The Phase 6 audit: one defect in three places, and a claim nothing checked](decisions/0174-the-phase-6-audit-one-defect-in-three-places-and-a-claim-nothing-checked.md) | Accepted | 2026-08-25 |
| [D-175 A break that does not compile is not a break](decisions/0175-a-break-that-does-not-compile-is-not-a-break.md) | Accepted | 2026-08-25 |
| [D-176 A boot target without the device is not a second boot target](decisions/0176-a-boot-target-without-the-device-is-not-a-second-boot-target.md) | Accepted | 2026-08-25 |
| [D-177 A check that names an address names a machine](decisions/0177-a-check-that-names-an-address-names-a-machine.md) | Accepted | 2026-08-25 |
| [D-178 Ask the device what it is, not what it is set to](decisions/0178-ask-the-device-what-it-is-not-what-it-is-set-to.md) | Accepted | 2026-08-25 |
| [D-179 The set the keyboard speaks is a fact about the controller](decisions/0179-the-set-the-keyboard-speaks-is-a-fact-about-the-controller.md) | Accepted | 2026-08-25 |
| [D-180 Wizzy gets the first native system-language/runtime proof](decisions/0180-wizzy-gets-the-first-native-system-language-runtime-proof.md) | Provisional | 2026-08-26 |
| [D-181 The POSIX register separates source truth from platform work](decisions/0181-the-posix-register-separates-source-truth-from-platform-work.md) | Accepted | 2026-08-26 |
| [D-182 A trap-served handle is paid for before the trap](decisions/0182-a-trap-served-handle-is-paid-for-before-the-trap.md) | Accepted | 2026-08-26 |
| [D-183 The Wizzy compiler gate precedes native-userland product work](decisions/0183-the-wizzy-compiler-gate-precedes-native-userland-product-work.md) | Accepted | 2026-08-26 |
| [D-184 The closed compiler gate has one finite, evidence-bound foundation register](decisions/0184-the-closed-compiler-gate-has-one-finite-evidence-bound-foundation-register.md) | Accepted | 2026-08-27 |
| [D-185 Shared mappings keep their frames, and mapping rollback owns only new entries](decisions/0185-shared-mappings-keep-their-frames-and-mapping-rollback-owns-only-new-entries.md) | Accepted | 2026-08-27 |
| [D-186 Supervisor writes obey read-only mappings by this kernel's act](decisions/0186-supervisor-writes-obey-read-only-mappings-by-this-kernels-act.md) | Accepted | 2026-08-27 |
| [D-187 PCI BAR discovery and function ownership are transactions](decisions/0187-pci-bar-discovery-and-function-ownership-are-transactions.md) | Accepted | 2026-08-27 |
| [D-188 Uncertain NVMe commands stop the controller or retain every DMA frame](decisions/0188-uncertain-nvme-commands-stop-the-controller-or-retain-every-dma-frame.md) | Accepted | 2026-08-27 |
| [D-189 The running address space exposes operations, never its mutable owner](decisions/0189-the-running-address-space-exposes-operations-never-its-mutable-owner.md) | Accepted | 2026-08-27 |
| [D-190 Page-table ownership grows before allocation, and cleanup refusal is data](decisions/0190-page-table-ownership-grows-before-allocation-and-cleanup-refusal-is-data.md) | Accepted | 2026-08-27 |
| [D-191 Process construction owns one rollback transaction until the thread is runnable](decisions/0191-process-construction-owns-one-rollback-transaction-until-the-thread-is-runnable.md) | Accepted | 2026-08-27 |
| [D-192 A terminal completion retains its operation until publication succeeds](decisions/0192-a-terminal-completion-retains-its-operation-until-publication-succeeds.md) | Accepted | 2026-08-27 |
| [D-193 An unguarded boot thread cannot open the user boundary](decisions/0193-an-unguarded-boot-thread-cannot-open-the-user-boundary.md) | Accepted | 2026-08-27 |
| [D-194 A forgotten spin-lock guard is an accepted, bounded liveness violation](decisions/0194-a-forgotten-spin-lock-guard-is-an-accepted-bounded-liveness-violation.md) | Accepted | 2026-08-27 |
| [D-195 NVMe reads cross the doorbell only as validated transfers](decisions/0195-nvme-reads-cross-the-doorbell-only-as-validated-transfers.md) | Accepted | 2026-08-27 |
| [D-196 Namespace Identify data is a validated 512-byte, metadata-free format before I/O queues exist](decisions/0196-namespace-identify-data-is-a-validated-512-byte-metadata-free-format-before-i-o.md) | Accepted | 2026-08-27 |
| [D-197 A refused PCI command enablement closes NVMe before a window exists](decisions/0197-a-refused-pci-command-enablement-closes-nvme-before-a-window-exists.md) | Accepted | 2026-08-27 |
| [D-198 Hardware-free namespaces do not depend on device discovery](decisions/0198-hardware-free-namespaces-do-not-depend-on-device-discovery.md) | Accepted | 2026-08-27 |
| [D-199 NVMe selection advances only from a completely retired candidate](decisions/0199-nvme-selection-advances-only-from-a-completely-retired-candidate.md) | Accepted | 2026-08-27 |
| [D-200 File reads leave trap context through one bounded worker, and NVMe waits use monotonic deadlines](decisions/0200-file-reads-leave-trap-context-through-one-bounded-worker-and-nvme-waits-use.md) | Accepted | 2026-08-27 |
| [D-201 Ring three opens only with enforced non-execution and a progressing clock](decisions/0201-ring-three-opens-only-with-enforced-non-execution-and-a-progressing-clock.md) | Accepted | 2026-08-28 |
| [D-202 The idle-start latch reserves an attempt and commits only a live thread](decisions/0202-the-idle-start-latch-reserves-an-attempt-and-commits-only-a-live-thread.md) | Accepted | 2026-08-28 |
| [D-203 Retired programs wake one mandatory service worker for bounded reclamation](decisions/0203-retired-programs-wake-one-mandatory-service-worker-for-bounded-reclamation.md) | Accepted | 2026-08-28 |
| [D-204 Handle-table ownership identities widen and refuse exhaustion](decisions/0204-handle-table-ownership-identities-widen-and-refuse-exhaustion.md) | Accepted | 2026-08-28 |
| [D-205 Every bounded I/O APIC is inventoried and one GSI has one owner](decisions/0205-every-bounded-i-o-apic-is-inventoried-and-one-gsi-has-one-owner.md) | Accepted | 2026-08-28 |
| [D-206 Hostile inputs end at typed refusal, while internal corruption remains fatal](decisions/0206-hostile-inputs-end-at-typed-refusal-while-internal-corruption-remains-fatal.md) | Accepted | 2026-08-28 |
| [D-207 A foreign FAT corpus is pinned as exact sparse sectors](decisions/0207-a-foreign-fat-corpus-is-pinned-as-exact-sparse-sectors.md) | Accepted | 2026-08-28 |
| [D-208 Mutation sweeps run from a journalled tracked-file copy](decisions/0208-mutation-sweeps-run-from-a-journalled-tracked-file-copy.md) | Accepted | 2026-08-28 |
| [D-209 A mutation is causal only across a green baseline and an exact new verdict](decisions/0209-a-mutation-is-causal-only-across-a-green-baseline-and-an-exact-new-verdict.md) | Accepted | 2026-08-28 |
| [D-210 Every pure crate keeps one load-bearing break alive](decisions/0210-every-pure-crate-keeps-one-load-bearing-break-alive.md) | Accepted | 2026-08-28 |
| [D-211 A boot image names and verifies the kernel it actually carries](decisions/0211-a-boot-image-names-and-verifies-the-kernel-it-actually-carries.md) | Accepted | 2026-08-28 |
| [D-212 The local gate is one registry rendered three ways](decisions/0212-the-local-gate-is-one-registry-rendered-three-ways.md) | Accepted | 2026-08-28 |
| [D-213 Stable rustc resolves the public foundation surface](decisions/0213-stable-rustc-resolves-the-public-foundation-surface.md) | Accepted | 2026-08-28 |
| [D-214 Hardware-retained pointers need irrevocable mappings, and unsafe review needs an exact revision](decisions/0214-hardware-retained-pointers-need-irrevocable-mappings-and-unsafe-review-needs-an.md) | Accepted | 2026-08-28 |
| [D-215 Operational Markdown is checked; historical numbers stay historical](decisions/0215-operational-markdown-is-checked-historical-numbers-stay-historical.md) | Accepted | 2026-08-28 |
| [D-216 Release evidence binds clean source and resolved inputs to reproducible, booted bytes](decisions/0216-release-evidence-binds-clean-source-and-resolved-inputs-to-reproducible-booted.md) | Accepted | 2026-08-28 |
| [D-217 Process birth and death evidence names one exact incarnation](decisions/0217-process-birth-and-death-evidence-names-one-exact-incarnation.md) | Accepted | 2026-08-29 |
| [D-218 Qualification names a stable machine and every resolved tool](decisions/0218-qualification-names-a-stable-machine-and-every-resolved-tool.md) | Accepted | 2026-08-29 |
| [D-219 Limine remains the boot boundary; current prose names current facts](decisions/0219-limine-remains-the-boot-boundary-current-prose-names-current-facts.md) | Accepted | 2026-08-29 |
| [D-220 The dependency record is offline; advisory freshness is exceptional](decisions/0220-the-dependency-record-is-offline-advisory-freshness-is-exceptional.md) | Accepted | 2026-08-29 |
| [D-221 A queue index is the synchronization edge, not a volatile payload](decisions/0221-a-queue-index-is-the-synchronization-edge-not-a-volatile-payload.md) | Accepted | 2026-08-29 |
| [D-222 A foreign handle has the same answer whether its slot is occupied or absent](decisions/0222-a-foreign-handle-has-the-same-answer-whether-its-slot-is-occupied-or-absent.md) | Accepted | 2026-08-29 |
| [D-223 CPU-local guards and syscall wakes have durable direct witnesses](decisions/0223-cpu-local-guards-and-syscall-wakes-have-durable-direct-witnesses.md) | Accepted | 2026-08-29 |
| [D-224 The boot memory map is complete or refused](decisions/0224-the-boot-memory-map-is-complete-or-refused.md) | Accepted | 2026-08-29 |
| [D-225 A retained PIT qualifies and can revoke public time](decisions/0225-a-retained-pit-qualifies-and-can-revoke-public-time.md) | Accepted | 2026-08-29 |
| [D-226 The Wizzy handoff is an exact route-neutral envelope](decisions/0226-the-wizzy-handoff-is-an-exact-route-neutral-envelope.md) | Accepted | 2026-08-29 |
| [D-227 QEMU runs own their writable state and every control endpoint](decisions/0227-qemu-runs-own-their-writable-state-and-every-control-endpoint.md) | Accepted | 2026-08-29 |
| [D-228 Extended-state proof follows every enabled component](decisions/0228-extended-state-proof-follows-every-enabled-component.md) | Accepted | 2026-08-29 |
| [D-229 The ring model carries visibility through every terminal owner](decisions/0229-the-ring-model-carries-visibility-through-every-terminal-owner.md) | Accepted | 2026-08-29 |
| [D-230 Foundation performance is a repeatable observation, not an ABI promise](decisions/0230-foundation-performance-is-a-repeatable-observation-not-an-abi-promise.md) | Accepted | 2026-08-29 |
| [D-231 A debugger owns one paused machine and discovers its endpoint](decisions/0231-a-debugger-owns-one-paused-machine-and-discovers-its-endpoint.md) | Accepted | 2026-08-29 |
| [D-232 Machine mutations get one bounded second observation after an inconclusive first](decisions/0232-machine-mutations-get-one-bounded-second-observation-after-an-inconclusive-first.md) | Accepted | 2026-08-29 |
| [D-233 Restoring mutation source invalidates its compiled package](decisions/0233-restoring-mutation-source-invalidates-its-compiled-package.md) | Accepted | 2026-08-29 |
| [D-234 A terminal observation keeps exact failures that preceded it](decisions/0234-a-terminal-observation-keeps-exact-failures-that-preceded-it.md) | Accepted | 2026-08-29 |
| [D-235 VMware evidence starts from two identified, separate machines](decisions/0235-vmware-evidence-starts-from-two-identified-separate-machines.md) | Accepted | 2026-08-29 |
| [D-236 Restoring a target dependency invalidates everything linked against it](decisions/0236-restoring-a-target-dependency-invalidates-everything-linked-against-it.md) | Accepted | 2026-08-29 |
| [D-237 A restored mutation is written, not copied](decisions/0237-a-restored-mutation-is-written-not-copied.md) | Accepted | 2026-08-29 |
| [D-238 A chain walk is bounded by the volume that could hold it](decisions/0238-a-chain-walk-is-bounded-by-the-volume-that-could-hold-it.md) | Accepted | 2026-08-29 |
| [D-239 The arriving bytes say which numbering they are in](decisions/0239-the-arriving-bytes-say-which-numbering-they-are-in.md) | Accepted | 2026-08-29 |
| [D-240 A round a human answers is bounded by their clock, and prompted on their screen](decisions/0240-a-round-a-human-answers-is-bounded-by-their-clock-and-prompted-on-their-screen.md) | Accepted | 2026-08-29 |
| [D-241 A described region is described by every region that holds it](decisions/0241-a-described-region-is-described-by-every-region-that-holds-it.md) | Accepted | 2026-08-29 |
| [D-242 A check is admissible when it cites a claim it did not read off the code](decisions/0242-a-check-is-admissible-when-it-cites-a-claim-it-did-not-read-off-the-code.md) | Accepted | 2026-08-29 |
| [D-243 Release authority is linear, and hostile layouts must be complete](decisions/0243-release-authority-is-linear-and-hostile-layouts-must-be-complete.md) | Accepted | 2026-08-30 |
| [D-244 Exceptional evidence records executions, not installed tools](decisions/0244-exceptional-evidence-records-executions-not-installed-tools.md) | Accepted | 2026-08-30 |
| [D-245 Cleanup authority survives every refusal, and partial construction is fail-atomic](decisions/0245-cleanup-authority-survives-every-refusal-and-partial-construction-is-fail-atomic.md) | Accepted | 2026-08-30 |
| [D-246 An independent grader judges frozen host logic](decisions/0246-an-independent-grader-judges-frozen-host-logic.md) | Accepted | 2026-08-30 |
| [D-247 Type errors and typed ownership make two review claims durable](decisions/0247-type-errors-and-typed-ownership-make-two-review-claims-durable.md) | Accepted | 2026-08-30 |
| [D-248 The shipped object surface stops at one opaque bootstrap identity](decisions/0248-the-shipped-object-surface-stops-at-one-opaque-bootstrap-identity.md) | Accepted | 2026-08-30 |
| [D-249 Device refusal is transactional, and internal contradictions fail in every build](decisions/0249-device-refusal-is-transactional-and-internal-contradictions-fail-in-every-build.md) | Accepted | 2026-08-30 |
| [D-250 Shared kernel mappings require positive runtime evidence](decisions/0250-shared-kernel-mappings-require-positive-runtime-evidence.md) | Accepted | 2026-08-30 |
| [D-251 Hardware observations mint exact, non-derivable authority](decisions/0251-hardware-observations-mint-exact-non-derivable-authority.md) | Accepted | 2026-08-30 |
| [D-252 A recoverable fault carries its one permitted destination](decisions/0252-a-recoverable-fault-carries-its-one-permitted-destination.md) | Accepted | 2026-08-30 |
| [D-253 Retained stack addresses are re-derived from permanent identity](decisions/0253-retained-stack-addresses-are-re-derived-from-permanent-identity.md) | Accepted | 2026-08-30 |
| [D-254 APIC initialization owns exclusion, not the caller's entry flag](decisions/0254-apic-initialization-owns-exclusion-not-the-callers-entry-flag.md) | Accepted | 2026-08-30 |
| [D-255 One arena guard owns both handle-slot choice and publication](decisions/0255-one-arena-guard-owns-both-handle-slot-choice-and-publication.md) | Accepted | 2026-08-30 |
| [D-256 Timer cleanup depends on admitted authority, and degraded panics are typed](decisions/0256-timer-cleanup-depends-on-admitted-authority-and-degraded-panics-are-typed.md) | Accepted | 2026-08-31 |
| [D-257 APIC readiness follows an exact software-enable readback](decisions/0257-apic-readiness-follows-an-exact-software-enable-readback.md) | Accepted | 2026-08-31 |
| [D-258 APIC deliverability is one shared typed decision](decisions/0258-apic-deliverability-is-one-shared-typed-decision.md) | Accepted | 2026-08-31 |
| [D-259 Retained stack identity is checked at map admission and hardware use](decisions/0259-retained-stack-identity-is-checked-at-map-admission-and-hardware-use.md) | Accepted | 2026-08-31 |
| [D-260 A panic becomes terminal evidence only at its framed completion record](decisions/0260-a-panic-becomes-terminal-evidence-only-at-its-framed-completion-record.md) | Accepted | 2026-08-31 |
| [D-261 Entry-slot identity owns translation eviction](decisions/0261-entry-slot-identity-owns-translation-eviction.md) | Accepted | 2026-08-31 |
| [D-262 Artifact limits are caller policy, not parser recovery](decisions/0262-artifact-limits-are-caller-policy-not-parser-recovery.md) | Accepted | 2026-08-31 |
| [D-263 Path authority is retained until publication or destruction commits](decisions/0263-path-authority-is-retained-until-publication-or-destruction-commits.md) | Accepted | 2026-08-31 |
| [D-264 A child is complete only after bounded exit and capture](decisions/0264-a-child-is-complete-only-after-bounded-exit-and-capture.md) | Accepted | 2026-08-31 |
| [D-265 One PCI function authority owns every uncertain transition](decisions/0265-one-pci-function-authority-owns-every-uncertain-transition.md) | Accepted | 2026-08-31 |
| [D-266 Unsafe instruction doors expose only their checked caller scope](decisions/0266-unsafe-instruction-doors-expose-only-their-checked-caller-scope.md) | Accepted | 2026-08-31 |
| [D-267 APIC completion and timer disarm are linear observations](decisions/0267-apic-completion-and-timer-disarm-are-linear-observations.md) | Accepted | 2026-08-31 |
| [D-268 Paging activation publishes `Ready` last](decisions/0268-paging-activation-publishes-ready-last.md) | Accepted | 2026-08-31 |
| [D-269 Shared memory chooses one access layout once](decisions/0269-shared-memory-chooses-one-access-layout-once.md) | Accepted | 2026-08-31 |
| [D-270 User admission asks one live extended-state predicate](decisions/0270-user-admission-asks-one-live-extended-state-predicate.md) | Accepted | 2026-08-31 |
| [D-271 Raw timebase parts must survive their published encoding](decisions/0271-raw-timebase-parts-must-survive-their-published-encoding.md) | Accepted | 2026-08-31 |
| [D-272 The Wizzy handoff has one line ending and one HTTPS authority grammar](decisions/0272-the-wizzy-handoff-has-one-line-ending-and-one-https-authority-grammar.md) | Accepted | 2026-08-31 |
| [D-273 An empty repeated MMIO access touches nothing](decisions/0273-an-empty-repeated-mmio-access-touches-nothing.md) | Accepted | 2026-08-31 |
| [D-274 A fresh advisory result binds one immutable database and one lock graph](decisions/0274-a-fresh-advisory-result-binds-one-immutable-database-and-one-lock-graph.md) | Accepted | 2026-08-31 |
| [D-275 Public reachability is bound to an exact compiler origin](decisions/0275-public-reachability-is-bound-to-an-exact-compiler-origin.md) | Accepted | 2026-09-01 |
| [D-276 An I/O APIC destination changes only behind a proven mask](decisions/0276-an-i-o-apic-destination-changes-only-behind-a-proven-mask.md) | Accepted | 2026-09-04 |
| [D-277 Interrupt sources share one typed global vector assignment](decisions/0277-interrupt-sources-share-one-typed-global-vector-assignment.md) | Accepted | 2026-09-04 |
| [D-278 PCI decode and DMA follow one function-bound BAR authority](decisions/0278-pci-decode-and-dma-follow-one-function-bound-bar-authority.md) | Accepted | 2026-09-04 |
| [D-279 User-copy recovery has one fixed test driver, not a general unchecked copy](decisions/0279-user-copy-recovery-has-one-fixed-test-driver-not-a-general-unchecked-copy.md) | Accepted | 2026-09-04 |
| [D-280 One address space mints one binding for one process lifetime](decisions/0280-one-address-space-mints-one-binding-for-one-process-lifetime.md) | Accepted | 2026-09-05 |
| [D-281 Kernel top-level topology freezes before user roots](decisions/0281-kernel-top-level-topology-freezes-before-user-roots.md) | Accepted | 2026-09-05 |
| [D-282 Hostile publication words are architecture memory, not Rust atomic objects](decisions/0282-hostile-publication-words-are-architecture-memory-not-rust-atomic-objects.md) | Accepted | 2026-09-05 |
| [D-283 Shared allocator aliases keep one write-back cache policy](decisions/0283-shared-allocator-aliases-keep-one-write-back-cache-policy.md) | Accepted | 2026-09-05 |
| [D-284 Shared top-level identity ignores only hardware access metadata](decisions/0284-shared-top-level-identity-ignores-only-hardware-access-metadata.md) | Accepted | 2026-09-05 |
| [D-285 Refused device retirement remains a reachable linear owner](decisions/0285-refused-device-retirement-remains-a-reachable-linear-owner.md) | Accepted | 2026-09-05 |
| [D-286 Repository-wide proofs fail closed on directory enumeration](decisions/0286-repository-wide-proofs-fail-closed-on-directory-enumeration.md) | Accepted | 2026-09-05 |
| [D-287 NVMe preparation preserves one linear owner across bounded stack stages](decisions/0287-nvme-preparation-preserves-one-linear-owner-across-bounded-stack-stages.md) | Accepted | 2026-09-05 |
| [D-288 Ownership proofs conserve allocator state and name one process incarnation](decisions/0288-ownership-proofs-conserve-allocator-state-and-name-one-process-incarnation.md) | Accepted | 2026-09-05 |
| [D-289 Every privilege entry installs the kernel stack selector before Rust](decisions/0289-every-privilege-entry-installs-the-kernel-stack-selector-before-rust.md) | Accepted | 2026-09-05 |
| [D-290 Machine evidence does not widen shipped hardware APIs](decisions/0290-machine-evidence-does-not-widen-shipped-hardware-apis.md) | Accepted | 2026-09-05 |
| [D-291 Generic trait dispatch is credited only at its exact declaration](decisions/0291-generic-trait-dispatch-is-credited-only-at-its-exact-declaration.md) | Accepted | 2026-09-06 |
| [D-292 The orchestrator binary is optimized; the graphs it audits are not](decisions/0292-the-orchestrator-binary-is-optimized-the-graphs-it-audits-are-not.md) | Accepted | 2026-09-06 |
| [D-293 Every static lock in `wost` is reclaimed by the panic path, and a contract says so](decisions/0293-every-static-lock-in-wost-is-reclaimed-by-the-panic-path-and-a-contract-says-so.md) | Accepted | 2026-09-06 |
| [D-294 The audited registry is rebound once per compiler round](decisions/0294-the-audited-registry-is-rebound-once-per-compiler-round.md) | Accepted | 2026-09-06 |
| [D-295 A type-qualified external-trait call is resolved by its callee type's one audited implementation](decisions/0295-a-type-qualified-external-trait-call-is-resolved-by-its-callee-types-one-audited.md) | Accepted | 2026-09-06 |
| [D-296 A refused BAR bind returns its reason, and a retained authority keeps no retry accessor](decisions/0296-a-refused-bar-bind-returns-its-reason-and-a-retained-authority-keeps-no-retry.md) | Accepted | 2026-09-06 |
| [D-297 The population run attests the advisory database; the pinned audit is bound by Git](decisions/0297-the-population-run-attests-the-advisory-database-the-pinned-audit-is-bound-by.md) | Accepted | 2026-09-06 |
| [D-298 Git pathname bytes precede filesystem path semantics](decisions/0298-git-pathname-bytes-precede-filesystem-path-semantics.md) | Accepted | 2026-09-10 |
| [D-299 Canonical paths stop at the GNU process boundary](decisions/0299-canonical-paths-stop-at-the-gnu-process-boundary.md) | Accepted | 2026-09-10 |
| [D-300 Qualification fixtures own their source-control premise](decisions/0300-qualification-fixtures-own-their-source-control-premise.md) | Accepted | 2026-09-10 |
| [D-301 A complete mutation report is a repair inventory, not a production verdict](decisions/0301-a-complete-mutation-report-is-a-repair-inventory-not-a-production-verdict.md) | Accepted | 2026-09-11 |
| [D-302 Redundant path defenses need layer-specific witnesses](decisions/0302-redundant-path-defenses-need-layer-specific-witnesses.md) | Accepted | 2026-09-12 |
| [D-303 Redundant capture bounds need phase-specific witnesses](decisions/0303-redundant-capture-bounds-need-phase-specific-witnesses.md) | Accepted | 2026-09-12 |
| [D-304 Qualification parser invariants need pre-record witnesses](decisions/0304-qualification-parser-invariants-need-pre-record-witnesses.md) | Accepted | 2026-09-12 |
| [D-305 Parent-substitution witnesses isolate the claimed verification layer](decisions/0305-parent-substitution-witnesses-isolate-the-claimed-verification-layer.md) | Accepted | 2026-09-12 |
| [D-306 Commit-time source witnesses pass every earlier record contract](decisions/0306-commit-time-source-witnesses-pass-every-earlier-record-contract.md) | Accepted | 2026-09-12 |
| [D-307 Source-order witnesses are confined to the executable body](decisions/0307-source-order-witnesses-are-confined-to-the-executable-body.md) | Accepted | 2026-09-12 |
| [D-308 Transcript witnesses preserve every contract except the one under test](decisions/0308-transcript-witnesses-preserve-every-contract-except-the-one-under-test.md) | Accepted | 2026-09-12 |
| [D-309 Semantic parser witnesses precede later shape validation](decisions/0309-semantic-parser-witnesses-precede-later-shape-validation.md) | Accepted | 2026-09-12 |
| [D-310 Mutation witnesses target one load-bearing predicate](decisions/0310-mutation-witnesses-target-one-load-bearing-predicate.md) | Accepted | 2026-09-12 |
| [D-311 Boundary witnesses fail at the named guard](decisions/0311-boundary-witnesses-fail-at-the-named-guard.md) | Accepted | 2026-09-12 |
| [D-312 A proof fixture reaches the predicate it names](decisions/0312-a-proof-fixture-reaches-the-predicate-it-names.md) | Accepted | 2026-09-12 |
| [D-313 Mutation witnesses observe the changed state](decisions/0313-mutation-witnesses-observe-the-changed-state.md) | Accepted | 2026-09-12 |
| [D-314 The userland is designed before its compiler, as a specification](decisions/0314-the-userland-is-designed-before-its-compiler-as-a-specification.md) | Accepted | 2026-09-12 |
| [D-315 One command registry serves every surface](decisions/0315-one-command-registry-serves-every-surface.md) | Accepted | 2026-09-12 |
| [D-316 Designation is authorization, and requirements and grants are two documents](decisions/0316-designation-is-authorization-and-requirements-and-grants-are-two-documents.md) | Accepted | 2026-09-12 |
| [D-317 The shell is typed and effect-previewed, and the terminal is a record protocol](decisions/0317-the-shell-is-typed-and-effect-previewed-and-the-terminal-is-a-record-protocol.md) | Accepted | 2026-09-12 |
| [D-318 The bench is a strip with snap semantics and a complete rail](decisions/0318-the-bench-is-a-strip-with-snap-semantics-and-a-complete-rail.md) | Accepted | 2026-09-12 |
| [D-319 The works are a static, verified, supervised and lazily started component graph](decisions/0319-the-works-are-a-static-verified-supervised-and-lazily-started-component-graph.md) | Accepted | 2026-09-12 |
| [D-320 Artifacts and packages are hash-addressed and reproducible, with generations, A/B images and a safe mode](decisions/0320-artifacts-and-packages-are-hash-addressed-and-reproducible-with-generations-a-b.md) | Accepted | 2026-09-12 |
| [D-321 One toolkit and one style source, with tokens as the OS API](decisions/0321-one-toolkit-and-one-style-source-with-tokens-as-the-os-api.md) | Accepted | 2026-09-12 |
| [D-322 Mutation qualification names compile-valid, causal terminal outcomes](decisions/0322-mutation-qualification-names-compile-valid-causal-terminal-outcomes.md) | Accepted | 2026-09-12 |
| [D-323 Linear failure wrappers announce retained authority](decisions/0323-linear-failure-wrappers-announce-retained-authority.md) | Accepted | 2026-09-14 |
| [D-324 The `xtask` independent grade fails fast under one bounded worker](decisions/0324-the-xtask-independent-grade-fails-fast-under-one-bounded-worker.md) | Accepted | 2026-09-27 |
| [D-325 Hostile-surface live classifications are checked in both directions](decisions/0325-hostile-surface-live-classifications-are-checked-in-both-directions.md) | Accepted | 2026-09-27 |
| [D-326 NVMe stack headroom is measured at the live register-access leaf](decisions/0326-nvme-stack-headroom-is-measured-at-the-live-register-access-leaf.md) | Accepted | 2026-09-27 |
| [D-327 A user-transition witness names one process incarnation](decisions/0327-a-user-transition-witness-names-one-process-incarnation.md) | Accepted | 2026-09-27 |
| [D-328 Deleted public APIs earn closure through exact compiler reintroduction](decisions/0328-deleted-public-apis-earn-closure-through-exact-compiler-reintroduction.md) | Accepted | 2026-09-27 |
| [D-329 Device publication and closure prose keep exact source authority](decisions/0329-device-publication-and-closure-prose-keep-exact-source-authority.md) | Accepted | 2026-09-28 |
| [D-330 External close records and one exact documentation child are replayable](decisions/0330-external-close-records-and-one-exact-documentation-child-are-replayable.md) | Accepted | 2026-09-28 |
| [D-331 External bytes clear before release, and mapping refusal retains their owner](decisions/0331-external-bytes-clear-before-release-and-mapping-refusal-retains-their-owner.md) | Accepted | 2026-09-28 |
| [D-332 Framebuffer scrolling keeps checked spans and interrupt progress](decisions/0332-framebuffer-scrolling-keeps-checked-spans-and-interrupt-progress.md) | Accepted | 2026-09-28 |
| [D-333 Final user entry is tested through the production xstate predicate](decisions/0333-final-user-entry-is-tested-through-the-production-xstate-predicate.md) | Accepted | 2026-09-28 |
| [D-334 Paging activation is one interrupt-excluded publication transaction](decisions/0334-paging-activation-is-one-interrupt-excluded-publication-transaction.md) | Accepted | 2026-09-28 |
| [D-335 PCI configuration changes consume one exact function authority](decisions/0335-pci-configuration-changes-consume-one-exact-function-authority.md) | Accepted | 2026-09-28 |
| [D-336 APIC initialization and completion each have one linear transaction](decisions/0336-apic-initialization-and-completion-each-have-one-linear-transaction.md) | Accepted | 2026-09-28 |
| [D-337 Mapping liveness follows the loaded root and frozen shared topology](decisions/0337-mapping-liveness-follows-the-loaded-root-and-frozen-shared-topology.md) | Accepted | 2026-09-28 |
| [D-338 Extended-state admission uses one exact live policy and a fresh initial image](decisions/0338-extended-state-admission-uses-one-exact-live-policy-and-a-fresh-initial-image.md) | Accepted | 2026-09-28 |
| [D-339 NVMe admission and DMA publication are causal ownership boundaries](decisions/0339-nvme-admission-and-dma-publication-are-causal-ownership-boundaries.md) | Accepted | 2026-09-28 |
| [D-340 Direct Cargo-subcommand probes name their entry point](decisions/0340-direct-cargo-subcommand-probes-name-their-entry-point.md) | Accepted | 2026-09-28 |
| [D-341 Mutation citations name permanent identities](decisions/0341-mutation-citations-name-permanent-identities.md) | Accepted | 2026-09-28 |
| [D-342 The `xtask` grade seals what cargo-mutants writes and witnesses its own completeness](decisions/0342-the-xtask-grade-seals-what-cargo-mutants-writes-and-witnesses-its-own.md) | Accepted | 2026-09-28 |
| [D-343 PCI function authority is counted at every door](decisions/0343-pci-function-authority-is-counted-at-every-door.md) | Accepted | 2026-09-28 |
| [D-344 A run trusts one Git, and a checked path is never reopened through a link](decisions/0344-a-run-trusts-one-git-and-a-checked-path-is-never-reopened-through-a-link.md) | Accepted | 2026-09-28 |
| [D-345 Supply-chain records refuse what their line readers do not model](decisions/0345-supply-chain-records-refuse-what-their-line-readers-do-not-model.md) | Accepted | 2026-09-28 |
| [D-346 QEMU control runs over the emulator's own standard streams](decisions/0346-qemu-control-runs-over-the-emulators-own-standard-streams.md) | Accepted | 2026-09-28 |
| [D-347 A bounded command ends with every process it started](decisions/0347-a-bounded-command-ends-with-every-process-it-started.md) | Accepted | 2026-09-28 |
| [D-348 No user mapping reaches the last canonical page](decisions/0348-no-user-mapping-reaches-the-last-canonical-page.md) | Accepted | 2026-09-29 |
| [D-349 Every RAM mapping keeps its direct-map cache type](decisions/0349-every-ram-mapping-keeps-its-direct-map-cache-type.md) | Accepted | 2026-09-29 |
| [D-350 The I/O APIC takeover proves the window is one before writing it](decisions/0350-the-i-o-apic-takeover-proves-the-window-is-one-before-writing-it.md) | Accepted | 2026-09-29 |
| [D-351 Fixed devices are claimed by name, not by port number](decisions/0351-fixed-devices-are-claimed-by-name-not-by-port-number.md) | Accepted | 2026-09-29 |
| [D-352 A directory walk is bounded by its entries and a cycle check, not the volume](decisions/0352-a-directory-walk-is-bounded-by-its-entries-and-a-cycle-check-not-the-volume.md) | Accepted | 2026-09-29 |
| [D-353 D-189 containment is proven by a signature inventory, and two validated tests land](decisions/0353-d-189-containment-is-proven-by-a-signature-inventory-and-two-validated-tests.md) | Accepted | 2026-09-29 |
| [D-354 Two fault witnesses name the incarnation that faulted](decisions/0354-two-fault-witnesses-name-the-incarnation-that-faulted.md) | Accepted | 2026-09-29 |
| [D-355 The busy-reap witness names the incarnation it reaped](decisions/0355-the-busy-reap-witness-names-the-incarnation-it-reaped.md) | Accepted | 2026-09-29 |
| [D-356 Queue impossibility backstops are terminal, and the reachable one is witnessed](decisions/0356-queue-impossibility-backstops-are-terminal-and-the-reachable-one-is-witnessed.md) | Accepted | 2026-09-29 |
| [D-357 DMA is isolated by an identity-scoped VT-d domain, and storage fails closed without one](decisions/0357-dma-is-isolated-by-an-identity-scoped-vt-d-domain-and-storage-fails-closed.md) | Accepted | 2026-09-29 |
| [D-358 The VT-d unit invalidates through registers, flushes tables for a non-coherent walker, and lives with the storage candidate](decisions/0358-the-vt-d-unit-invalidates-through-registers-flushes-tables-for-a-non-coherent.md) | Accepted | 2026-10-01 |
| [D-359 The storage candidate is sequenced through sibling stages, so one stage frame at a time sits on the boot stack](decisions/0359-the-storage-candidate-is-sequenced-through-sibling-stages-so-one-stage-frame-at.md) | Accepted | 2026-10-03 |
| [D-360 Reserved DMA regions stay clear of kernel memory, the domain proof is spent once, and four source checks close their evasions](decisions/0360-reserved-dma-regions-stay-clear-of-kernel-memory-the-domain-proof-is-spent-once.md) | Accepted | 2026-10-03 |
| [D-361 The NVMe stack meter paints the boot stack across each storage candidate, and its scan answers to the live leaf](decisions/0361-the-nvme-stack-meter-paints-the-boot-stack-across-each-storage-candidate-and-its.md) | Accepted | 2026-10-03 |
| [D-362 The remaining manifest and macro source-check evasions fail closed](decisions/0362-the-remaining-manifest-and-macro-source-check-evasions-fail-closed.md) | Accepted | 2026-10-07 |
| [D-363 Historical handoff dispositions follow their finite owners](decisions/0363-historical-handoff-dispositions-follow-their-finite-owners.md) | Accepted | 2026-10-07 |
| [D-364 VT-d table depth and effective address coverage are independent](decisions/0364-vt-d-table-depth-and-effective-address-coverage-are-independent.md) | Accepted | 2026-10-07 |
| [D-365 VMware qualification binds configuration and preserves runtime bytes](decisions/0365-vmware-qualification-binds-configuration-and-preserves-runtime-bytes.md) | Accepted | 2026-10-07 |
| [D-366 Bedrock owns the kernel and Realm owns the operating environment](decisions/0366-bedrock-owns-the-kernel-and-realm-owns-the-operating-environment.md) | Accepted | 2026-10-07 |
| [D-367 Uncertain device reachability consumes ownership, and hostile diagnostics are bounded](decisions/0367-uncertain-device-reachability-consumes-ownership-and-hostile-diagnostics-are.md) | Accepted | 2026-10-09 |
| [D-368 Every ring-three return is proved safe at the last common boundary](decisions/0368-every-ring-three-return-is-proved-safe-at-the-last-common-boundary.md) | Accepted | 2026-10-09 |
| [D-369 Rejected storage candidates retire behind a verified DMA fence](decisions/0369-rejected-storage-candidates-retire-behind-a-verified-dma-fence.md) | Accepted | 2026-10-09 |
| [D-370 Unit-wide VT-d transitions own and drain every declared requester](decisions/0370-unit-wide-vt-d-transitions-own-and-drain-every-declared-requester.md) | Accepted | 2026-10-09 |
| [D-371 DMAR authority comes from one complete sealed PCI topology](decisions/0371-dmar-authority-comes-from-one-complete-sealed-pci-topology.md) | Accepted | 2026-10-09 |
| [D-372 PCI interrupts are default-deny behind VT-d remapping](decisions/0372-pci-interrupts-are-default-deny-behind-vt-d-remapping.md) | Accepted | 2026-10-09 |
| [D-373 Qualification PASS requires an owned execution or authenticated provider receipt](decisions/0373-qualification-pass-requires-an-owned-execution-or-authenticated-provider-receipt.md) | Accepted | 2026-10-09 |
| [D-374 A pending-vector witness names its request independently of APIC priority](decisions/0374-a-pending-vector-witness-names-its-request-independently-of-apic-priority.md) | Accepted | 2026-10-09 |
| [D-375 Local APIC takeover masks every advertised source before publication](decisions/0375-local-apic-takeover-masks-every-advertised-source-before-publication.md) | Accepted | 2026-10-10 |
| [D-376 Dynamic mapping admission rejects W+X before page-table preflight](decisions/0376-dynamic-mapping-admission-rejects-w-x-before-page-table-preflight.md) | Accepted | 2026-10-10 |
| [D-377 CI reads its registry document through the bounded direct-file boundary](decisions/0377-ci-reads-its-registry-document-through-the-bounded-direct-file-boundary.md) | Accepted | 2026-10-10 |
| [D-378 A polled-reader witness waits for both observed consequences](decisions/0378-a-polled-reader-witness-waits-for-both-observed-consequences.md) | Accepted | 2026-10-10 |
| [D-379 Boot-image sources are bounded at their consuming copy](decisions/0379-boot-image-sources-are-bounded-at-their-consuming-copy.md) | Accepted | 2026-10-10 |
| [D-380 ESP population authenticates the exact bootloader buffer](decisions/0380-esp-population-authenticates-the-exact-bootloader-buffer.md) | Accepted | 2026-10-10 |
| [D-381 OpenAt transfers storage work out of trap context with its authority intact](decisions/0381-openat-transfers-storage-work-out-of-trap-context-with-its-authority-intact.md) | Accepted | 2026-10-10 |
| [D-382 Destructive cleanup retains directory authority through traversal](decisions/0382-destructive-cleanup-retains-directory-authority-through-traversal.md) | Accepted | 2026-10-10 |
| [D-383 VT-d revocation drains older DMA before returning frame ownership](decisions/0383-vt-d-revocation-drains-older-dma-before-returning-frame-ownership.md) | Accepted | 2026-10-10 |
| [D-384 The foundation baseline follows reviewed security state, not its historical footprint](decisions/0384-the-foundation-baseline-follows-reviewed-security-state-not-its-historical.md) | Accepted | 2026-10-11 |
| [D-385 Qualification command fixtures do not inherit the checkout that compiled them](decisions/0385-qualification-command-fixtures-do-not-inherit-the-checkout-that-compiled-them.md) | Accepted | 2026-10-11 |
| [D-386 Shipped debug information names stable virtual roots](decisions/0386-shipped-debug-information-names-stable-virtual-roots.md) | Accepted | 2026-10-11 |
| [D-387 Live VT-d entries publish once and NVMe mappings carry direction](decisions/0387-live-vt-d-entries-publish-once-and-nvme-mappings-carry-direction.md) | Accepted | 2026-10-11 |
| [D-388 Firmware-reserved DMA regions carry no mapping authority](decisions/0388-firmware-reserved-dma-regions-carry-no-mapping-authority.md) | Accepted | 2026-10-11 |
| [D-389 VT-d stays default-deny across topology discovery and controller fallback](decisions/0389-vt-d-stays-default-deny-across-topology-discovery-and-controller-fallback.md) | Accepted | 2026-10-11 |
| [D-390 DMA safety is a verified boot-handoff invariant](decisions/0390-dma-safety-is-a-verified-boot-handoff-invariant.md) | Accepted | 2026-10-11 |
| [D-391 API audit keeps structured evidence inside its fixed capture ceiling](decisions/0391-api-audit-keeps-structured-evidence-inside-its-fixed-capture-ceiling.md) | Accepted | 2026-10-12 |
| [D-392 VMware's exact 440BX host bridge is present but is not a DMA requester](decisions/0392-vmwares-exact-440bx-host-bridge-is-present-but-is-not-a-dma-requester.md) | Accepted | 2026-10-12 |
| [D-393 API audit reduces compiler evidence while it streams](decisions/0393-api-audit-reduces-compiler-evidence-while-it-streams.md) | Accepted | 2026-10-12 |
| [D-394 Mutation witnesses follow the current causal verdict](decisions/0394-mutation-witnesses-follow-the-current-causal-verdict.md) | Accepted | 2026-10-12 |
| [D-395 Mutation completion witnesses exercise reachable behavior and the earliest causal verdict](decisions/0395-mutation-completion-witnesses-exercise-reachable-behavior-and-the-earliest.md) | Accepted | 2026-10-13 |
| [D-396 Corrupt-state witnesses isolate one queue invariant at a time](decisions/0396-corrupt-state-witnesses-isolate-one-queue-invariant-at-a-time.md) | Accepted | 2026-10-13 |
| [D-397 A forgotten EOI token names the exclusion it strands](decisions/0397-a-forgotten-eoi-token-names-the-exclusion-it-strands.md) | Accepted | 2026-10-13 |
| [D-398 A redirect mutation removes the complete pre-read refusal](decisions/0398-a-redirect-mutation-removes-the-complete-pre-read-refusal.md) | Accepted | 2026-10-13 |
| [D-399 A quarantined VT-d root names the scheduling failure it causes](decisions/0399-a-quarantined-vt-d-root-names-the-scheduling-failure-it-causes.md) | Accepted | 2026-10-13 |
| [D-400 Stack-margin mutations are calibrated against each qualified profile](decisions/0400-stack-margin-mutations-are-calibrated-against-each-qualified-profile.md) | Accepted | 2026-10-13 |
| [D-401 Interrupt-boundary source contracts prove duplicate guards and live predicates](decisions/0401-interrupt-boundary-source-contracts-prove-duplicate-guards-and-live-predicates.md) | Accepted | 2026-10-13 |
| [D-402 Dead-API mutations must reach the API auditor](decisions/0402-dead-api-mutations-must-reach-the-api-auditor.md) | Accepted | 2026-10-13 |
| [D-403 Compiler identity is authenticated without resident executable-sized allocations](decisions/0403-compiler-identity-is-authenticated-without-resident-executable-sized-allocations.md) | Accepted | 2026-10-13 |
| [D-404 API-audit compiler graphs have one source-controlled Cargo job](decisions/0404-api-audit-compiler-graphs-have-one-source-controlled-cargo-job.md) | Accepted | 2026-10-13 |
| [D-405 Every compiler-authority file is authenticated with bounded resident memory](decisions/0405-every-compiler-authority-file-is-authenticated-with-bounded-resident-memory.md) | Accepted | 2026-10-13 |
| [D-406 Codex Security Deep PASS is owned by one authenticated live MCP session](decisions/0406-codex-security-deep-pass-is-owned-by-one-authenticated-live-mcp-session.md) | Accepted | 2026-10-14 |
| [D-407 Native user execution without RDCL_NO uses paired roots and an exact entry enclave](decisions/0407-native-user-execution-without-rdcl-no-uses-paired-roots-and-an-exact-entry.md) | Accepted | 2026-10-14 |
| [D-408 Windows canonical path spelling cannot reject the owned Deep qualifier](decisions/0408-windows-canonical-path-spelling-cannot-reject-the-owned-deep-qualifier.md) | Accepted | 2026-10-15 |
| [D-409 Codex login status may use stderr](decisions/0409-codex-login-status-may-use-stderr.md) | Accepted | 2026-10-15 |
| [D-410 Codex initialization metadata is exact and home-bound](decisions/0410-codex-initialization-metadata-is-exact-and-home-bound.md) | Accepted | 2026-10-15 |
| [D-411 Codex initialization identity follows the hermetic client](decisions/0411-codex-initialization-identity-follows-the-hermetic-client.md) | Accepted | 2026-10-15 |
| [D-412 Codex full-schema defaults and origins are closed](decisions/0412-codex-full-schema-defaults-and-origins-are-closed.md) | Accepted | 2026-10-15 |
| [D-413 Runtime-generated Codex system skills are byte-bound](decisions/0413-runtime-generated-codex-system-skills-are-byte-bound.md) | Accepted | 2026-10-15 |
| [D-414 Paging self-test ownership is measured in one exclusion epoch](decisions/0414-paging-self-test-ownership-is-measured-in-one-exclusion-epoch.md) | Accepted | 2026-10-15 |
| [D-415 Safety-derived null defaults are normalized exactly](decisions/0415-safety-derived-null-defaults-are-normalized-exactly.md) | Accepted | 2026-10-15 |
| [D-416 The owned Deep dialogue gives Daybreak one advisory read before scan start](decisions/0416-the-owned-deep-dialogue-gives-daybreak-one-advisory-read-before-scan-start.md) | Accepted | 2026-10-16 |
| [D-417 The owned Deep runtime advances only as one remeasured exact tuple](decisions/0417-the-owned-deep-runtime-advances-only-as-one-remeasured-exact-tuple.md) | Accepted | 2026-10-17 |
| [D-418 Windows cleanup admits only its exact live executable](decisions/0418-windows-cleanup-admits-only-its-exact-live-executable.md) | Accepted | 2026-10-17 |
| [D-419 Generated system-skill identity uses the verifier's ordinal path order](decisions/0419-generated-system-skill-identity-uses-the-verifiers-ordinal-path-order.md) | Accepted | 2026-10-17 |
| [D-420 Deep workers use an owned default-deny source clone](decisions/0420-deep-workers-use-an-owned-default-deny-source-clone.md) | Accepted | 2026-10-17 |
| [D-421 VMware delayed-read verifier follows the kernel format literal](decisions/0421-vmware-delayed-read-verifier-follows-the-kernel-format-literal.md) | Accepted | 2026-10-17 |
| [D-422 Deep separates the authenticated control-plane envelope from the effective default-deny worker profile](decisions/0422-deep-separates-the-authenticated-control-plane-envelope-from-the-effective.md) | Accepted | 2026-10-18 |
| [D-423 The active Codex tuple and normalized default projection are remeasured together](decisions/0423-the-active-codex-tuple-and-normalized-default-projection-are-remeasured-together.md) | Accepted | 2026-10-18 |
| [D-424 Deep separately proves existing-file mutation and outbound connection denial](decisions/0424-deep-separately-proves-existing-file-mutation-and-outbound-connection-denial.md) | Accepted | 2026-10-18 |
| [D-425 Bedrock releases follow kernel practice and the grade and Deep run continuously](decisions/0425-bedrock-releases-follow-kernel-practice-and-the-grade-and-deep-run-continuously.md) | Accepted | 2026-10-19 |
| [D-426 The first real VMware boots set the platform's ISA, capacity, descriptor and transcript facts](decisions/0426-the-first-real-vmware-boots-set-the-platforms-isa-capacity-descriptor-and.md) | Accepted | 2026-10-20 |
| [D-427 Shipped liveness requires a call from ring 3](decisions/0427-shipped-liveness-requires-a-call-from-ring-3.md) | Accepted | 2026-10-23 |
| [D-428 The ISA DMA proof checks the adapter operations as well as their caller](decisions/0428-the-isa-dma-proof-checks-the-adapter-operations-as-well-as-their-caller.md) | Accepted | 2026-10-23 |
| [D-429 A complete independent grade matches its exit to adjudicated outcomes](decisions/0429-a-complete-independent-grade-matches-its-exit-to-adjudicated-outcomes.md) | Accepted | 2026-10-23 |
| [D-430 Leaf callback review follows authority, not a stale static count](decisions/0430-leaf-callback-review-follows-authority-not-a-stale-static-count.md) | Accepted | 2026-10-23 |
| [D-431 VMware console automation admits one complete loopback runtime group](decisions/0431-vmware-console-automation-admits-one-complete-loopback-runtime-group.md) | Accepted | 2026-10-23 |
| [D-432 A failed PCI observation revokes its snapshot until reset](decisions/0432-a-failed-pci-observation-revokes-its-snapshot-until-reset.md) | Accepted | 2026-10-23 |
| [D-433 DMA ownership survives safe owner destruction](decisions/0433-dma-ownership-survives-safe-owner-destruction.md) | Accepted | 2026-10-23 |
| [D-434 The attenuation-refusal witness reaches attenuation](decisions/0434-the-attenuation-refusal-witness-reaches-attenuation.md) | Accepted | 2026-10-23 |
| [D-435 QEMU cleanup and QMP command waits have finite failure paths](decisions/0435-qemu-cleanup-and-qmp-command-waits-have-finite-failure-paths.md) | Accepted | 2026-10-23 |
| [D-436 Mutation readers open checked real-file handles](decisions/0436-mutation-readers-open-checked-real-file-handles.md) | Accepted | 2026-10-23 |
| [D-437 Known CLI options belong to the task that consumes them](decisions/0437-known-cli-options-belong-to-the-task-that-consumes-them.md) | Accepted | 2026-10-23 |
| [D-438 A refused storage-input wait releases its held command](decisions/0438-a-refused-storage-input-wait-releases-its-held-command.md) | Accepted | 2026-10-23 |
| [D-439 Each keyboard batch retains producer order through admission](decisions/0439-each-keyboard-batch-retains-producer-order-through-admission.md) | Accepted | 2026-10-23 |
| [D-440 The stitched-record witness stitches records only line splitting can see](decisions/0440-the-stitched-record-witness-stitches-records-only-line-splitting-can-see.md) | Accepted | 2026-10-24 |
| [D-441 Experimental source previews have a separate publication contract](decisions/0441-experimental-source-previews-have-a-separate-publication-contract.md) | Accepted | 2026-10-25 |
| [D-442 API audit graphs share bounded workers and retain ordered authority](decisions/0442-api-audit-graphs-share-bounded-workers-and-retain-ordered-authority.md) | Accepted | 2026-10-26 |
| [D-443 Clock resolution is observed between adjacent counter reads](decisions/0443-clock-resolution-is-observed-between-adjacent-counter-reads.md) | Accepted | 2026-10-26 |
| [D-444 VMware parser regressions retain captured guest records](decisions/0444-vmware-parser-regressions-retain-captured-guest-records.md) | Accepted | 2026-10-26 |
| [D-445 Foundation resource bands follow attributed security ownership](decisions/0445-foundation-resource-bands-follow-attributed-security-ownership.md) | Accepted | 2026-10-26 |
| [D-446 An admitted self-test wait stops when its clock is revoked](decisions/0446-an-admitted-self-test-wait-stops-when-its-clock-is-revoked.md) | Accepted | 2026-10-26 |
| [D-447 Mutation 2,432 removes every unavailable-clock refusal](decisions/0447-mutation-2-432-removes-every-unavailable-clock-refusal.md) | Accepted | 2026-10-27 |
| [D-448 The first qualified release is v0.2.0 and the SBOM follows the workspace version](decisions/0448-the-first-qualified-release-is-v0-2-0-and-the-sbom-follows-the-workspace-version.md) | Accepted | 2026-10-27 |
| [D-449 v0.2.0 qualifies `694061f` and closes the rows its gate proved](decisions/0449-v0-2-0-qualifies-694061f-and-closes-the-rows-its-gate-proved.md) | Accepted | 2026-10-28 |
| [D-450 Linux hosts run the whole gate](decisions/0450-linux-hosts-run-the-whole-gate.md) | Accepted | 2026-11-07 |
| [D-451 Decisions, documentation and code voice](decisions/0451-decisions-documentation-and-code-voice.md) | Accepted | 2026-11-09 |
| [D-452 The roadmap after the foundation close](decisions/0452-the-roadmap-after-the-foundation-close.md) | Accepted | 2026-11-09 |
| [D-453 Worlds are the root of the roadmap](decisions/0453-worlds-are-the-root-of-the-roadmap.md) | Accepted | 2026-11-14 |
