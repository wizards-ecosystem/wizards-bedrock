# Roadmap

This document orders Bedrock's kernel work and fixes the design choices the order rests on.
Ownership follows D-366: Bedrock owns kernel mechanisms, the native ABI, drivers and
qualification; Realm owns the operating environment; Ink owns Wizzy. Each phase below ships with
its own witness programs, written in Rust and retired when Realm's Wizzy programs replace them;
they are checks, not a userland, so D-183's rule that first-party product software is Wizzy
stands. Every ABI change is versioned, so a witness never freezes an interface. A kernel
mechanism does not wait for a Wizzy caller. Section 7 records what exists in the code today
against this plan.

## 1. Position

Everything runs in a world, and a world is something the kernel can hold.
A world is a nesting group of processes together with everything it owns:
its memory and the kernel's records about it, its handles, its rings, its
clock and randomness, its budget and its child worlds. The kernel creates,
freezes, copies, moves, rewinds and destroys a world as one object, and
knows nothing about the files, programs or operating systems inside it.

Five rules hold from the first phase:

1. **A world owns everything it uses.** Every kernel byte a world causes
   lives in that world's arena, so a world is copied, moved or destroyed
   whole and accounted exactly.
2. **Everything that enters a world passes through the kernel.** Device
   data arrives as ring completions, and time and randomness are served by
   the kernel unless a world is granted raw access, so a world can be
   recorded, rewound and kept alive on another machine.
3. **A world has only what it was handed.** No ambient authority: a world
   starts with nothing and reaches only the handles it was given.
4. **The walls are real and proven.** Isolation between worlds is the
   process boundary on a kernel small enough to prove.
5. **Fast by design.** Worlds reach devices and each other through
   per-core queues and shared memory, with no kernel entry on the steady
   data path where the processor allows.

Bedrock is the small, provably isolating base under servers. A tenant is a
world, made cheap enough that thousands fit on one machine. Bedrock runs on
x86_64 hardware. Realm runs on Bedrock as the world manager and the
operating environment; on servers it runs headless behind an API. Linux and
other software run inside worlds: Linux is hosted, never imitated. Bedrock
takes the place of the hypervisor under Linux and also runs native worlds
directly. Wizzy is the policy and extension language and the control plane.
The laptop is the owner's showcase; nothing here claims a general-purpose
consumer desktop.

The design aims past the current state of the art at each layer:

| Layer | What Bedrock adds |
|---|---|
| Proof | A proof of the kernel that covers its fine-grained locking across processors |
| Memory | Arenas per world and cache-colour-aware frames from the first allocator |
| Worlds | Copy of a whole running world, kernel state and live rings included, in under a millisecond |
| Time | One kernel-owned clock that serves replay, timing protection and the proofs at once |
| Devices | Device queues as capabilities, fenced restartable drivers, completions without kernel entry where the processor allows |
| Extensions | A proven checker for kernel extensions in place of a symbolic verifier |
| Durability | A world that survives the loss of its machine, unmodified, with replies held only until a standby has its inputs |
| Networking | A durable world carries its network state in its own memory, so freezing or moving it keeps its connections |
| Wizzy | A neutral ABI that Wizzy's types express exactly |

## 2. Design choices fixed here

| Choice | Decision |
|---|---|
| Program loading | A user-space loader maps segments into a new address space and starts a thread. The kernel parses no executable format. Spawn takes a sealed, execute-bearing memory object (section 2, code loading) |
| IPC | The asynchronous ring (D-06) plus a synchronous call and reply path that switches directly from caller to server. Channels carry bytes and handles (D-110). A reply right is used once |
| Worlds | Processes live in kernel-enforced worlds that nest. Policy, limits and kill attach to a world. Each world keeps a flat set of its descendants rather than a tree, the layout that keeps the proof affordable |
| Kernel memory | Each world owns an arena of frames its parent donated at spawn or later. Every kernel object the world creates is carved from its arena by the kernel's own allocator; object sizes stay private; a dead world's arena is freed whole after the grace period. The kernel never allocates from a shared pool for a user request. Arenas are copy-on-write clonable from the first version |
| Cache colouring | The frame allocator knows each frame's cache colour from day one, kernel frames included. Policy decides per world: strangers sharing cores get disjoint colours; trusted or demanding worlds get huge pages and the whole cache |
| Clock and randomness | A world never touches the hardware clock or random-number instructions unless granted. Each world runs in a mode: normal (a fast kernel clock page at a precision the kernel sets, the default), recorded (served and logged, for replay and durability) or raw (a granted right; the world is then neither replayable nor timing protected). A copied world receives fresh randomness. Handle numbers and address layout come from a per-world seed |
| Multi-core | Application processors start right after the process and IPC basics, with fine-grained locking from the first bring-up. This replaces the earlier plan of a late multi-core phase that started from one big kernel lock |
| Drivers | Native drivers exist only where speed is the product: NVMe, the server NICs and the virtual devices of clouds and VMs. A native driver is safe Rust that reaches hardware only through tokens `wost` issues (D-13), sharded per core with one queue pair, one vector and one single-producer ring per core and no shared mutable state on the data path. It runs as a fenced process with its own address space, IOMMU domain and budget; small latency-sensitive drivers may run inside a budgeted kernel executor from the same source. Rings and buffers belong to the client, so a restarted driver is reconnected, not repaired. The intent log of submitted-but-incomplete operations is part of the ring contract, so storage restart is reset and resubmit. All other hardware is passed to a driver world: a Linux guest whose own drivers run the device behind the IOMMU and restart without touching the kernel |
| Network stack | A program on top of the kernel, never inside it. The kernel hands out device queues; a stack service holds the protocol core, which stays a platform-independent crate so a world can carry its own stack over its own queue with no interface change. Programs see one socket interface whatever the home. Queues between driver, stack and program are batched shared memory, which is what sets the per-packet cost. There is no in-kernel network stack, and the driver order stays virtio-net, then e1000 |
| Kernel extensions | Wizzy programs run in kernel hook points (driver ingress, ring filters, every rights derivation, world lifecycle, scheduling, tracepoints). The kernel holds only a type, effect and linearity checker for a small typed intermediate form and a tested emitter; no symbolic verifier and no compiler. Running time is bounded in the form's types. eBPF bytecode is accepted through a second front end that lifts into the same form and fails closed |
| Wizzy | Wizzy is the policy and extension language and the control plane. The native ABI stays language neutral, and no ABI feature is accepted unless Wizzy's types express it exactly. Bedrock and Wizzy stay independent and meet at a published core contract that neither owns and each implements; Rust uses the ABI through safe wrappers |
| Tenancy | A cheap world per tenant is the product; nesting lets a tenant spawn sub-tenants with fewer rights. Worlds of one owner may share a process as a research track (section 6) |
| Code loading | Write, seal, execute: seal consumes the handle and returns a new one, there is no unseal and no dual mapping, code generation is a transferable quota object, executable placement is randomized, and seal refuses while any live write reference exists |
| Storage | A content-addressed object store on raw NVMe comes first and holds world snapshots, disks and images. WizFS (D-08) is built on it later |
| Linux | Linux runs as a guest inside a world. A box is booted once, frozen and cloned copy-on-write for each tenant, so a box costs only the memory it changes. Worlds of one owner may share a box; strangers never do. A bounded runner for static Linux binaries waits for evidence (section 7) |
| Proof | Each core piece carries a short written statement of what must be true. Proofs in Verus are written and checked at the milestones in section 3, against those statements. Between milestones a phase keeps one witness, a boot smoke check and a focused test for each real bug fixed |
| Architecture | x86_64 only. The `arch/` seams stay so this remains a choice. The capability format keeps spare fields for pointer-capability hardware and shared memory pools |
| Hardware | Real hardware waits until the mechanisms exist. QEMU is the loop and VMware the milestone (D-18) through every phase before the laptop phase |

## 3. Phase order

```text
  7  Processes, worlds and arenas          witness: init, supervisor, echo
  8  Multi-core                            witness: the same on 2 and 4 CPUs
     milestone 1: arenas, handles and memory maps; locking modelled
  9  Drivers as processes                  witness: virtio and NVMe fenced
 10  Storage write path and object store   witness: snapshot image round trip
 11  Worlds as values                      witness: clone 16, resume, diff
     milestone 2: drivers, the store, copy and freeze
 12  Linux boxes                           witness: a booted box cloned
     milestone 3: the guest boundary
 13  Networking                            witness: echo over the fenced NIC
 14  Extensions and the checker            witness: filter and rights policy
 15  Replay                                witness: record once, replay 100
 16  Durable worlds                        witness: power off mid-stream
     milestone 4: networking, the checker, replay, the commit rule, locking
 17  Schedulers and the runtime contract   witness: tail latency
 18  Operability                           witness: crash log read offline
 19  The laptop showcase                   witness: Realm on the HP ENVY
 20  WizFS                                 witness: crash injection, repair
     milestone 5: everything since milestone 4
Research tracks run beside 9 onward (section 6).
```

Each phase closes with its witness, a QEMU boot smoke check and a VMware
boot (D-18); a phase that adds concurrency adds its model (D-84). A
milestone closes when the proofs of everything built since the last one
check against their written statements, and what they find is fixed before
the next phase starts. A new check needs no mutation run (D-454).

## 4. Mechanisms by phase

| Phase | Mechanism | Witness |
|---|---|---|
| 7 | Worlds with policy, limits and kill; flat descendant sets; per-world arenas, clonable and colour-aware; process object; spawn from a sealed memory object; exit status and child observation | A Rust supervisor spawns one child that exits 0 and one that faults, observes both, and a third that over-fills its arena is refused without touching its sibling |
| 7 | Channels with handle transfer; versioned bootstrap message; the call and reply path with single-use reply rights | An echo service answers through both paths; a handoff witness transfers every endpoint to a successor with no message lost or duplicated |
| 7 | Memory objects: map, unmap with a flush token, write-then-seal, execute right | Seal with a writable mapping, map executable without the right and mint the right without the capability are each refused |
| 7 | CSPRNG from `RDSEED`; the clock page and the trap on direct clock reads; per-world seeds; the log record format that replay and durability use; spare capability fields; directory enumeration; console and input device handles; user-space loader | The loader starts the supervisor from the image; `ls` over the ring; a direct clock read from a world without the right is refused |
| 8 | Application processors through Limine's request; per-processor stacks, tables, speculation controls and timer; per-processor run queues on `libs/eevdf`; TLB shootdown; PCID under paired roots (D-250) | TLA+ models of shootdown and the lost-wakeup re-check join `cargo xtask model-check`; no lock from Phase 7 is replaced by a global one |
| 9 | Driver tokens in `wost`; per-core shards; client-owned rings; one IOMMU domain per driver with static mappings; generation-tagged reconnect; the intent log | virtio-net and virtio-blk, then NVMe and e1000e, as fenced processes against the in-kernel executor on four processors: within ten percent at batch 32; kill the driver once a second under a write stream, recover in under 50 ms with no acknowledged write lost |
| 10 | NVMe write path; tmpfs mutation; the content-addressed object store with copy-on-write chunks | An image written, deduplicated and read back by digest; FAT32 stays read-only |
| 11 | World snapshot to a handle; clone; freeze; resume; ring quiescence; kernel-reset clone identity and fresh randomness; a working-set record in each snapshot for prefetch on resume | A four-process world with open rings is snapshotted and cloned sixteen times concurrently; each clone under a millisecond, resume from the store under ten; a reused slot never reads its predecessor's pages |
| 12 | A virtual processor as a thread attribute and extended page tables as an address-space variant; virtio backends on Phase 9's fenced drivers; boot once and clone; one box per trust group; device passthrough for driver worlds | An unmodified Linux guest boots on QEMU and VMware with virtio network and block; a booted box is cloned sixteen times and each clone runs without booting; clone time is recorded against a microVM snapshot restore on the same machine |
| 13 | NIC queues as capabilities; the stack service over the fenced e1000e with one socket interface; tenant fairness; MSI-X through remapping (D-372); user interrupts for completions where present, ring wake otherwise; vmxnet3 and cloud NICs behind named deployments | Echo through the stack service on QEMU and VMware; the handoff witness repeats with a listening socket; two hostile tenants share one NIC and neither reads the other's frames; kernel entries and copies per packet are counted and recorded |
| 14 | The typed intermediate form with bounded running time, its checker and emitter; hook points; eBPF front end | A packet filter and a rights policy in Wizzy beside their eBPF twins; the checker's typing rules written for the milestone proof |
| 15 | Recorded mode; the completion log in a format that a standby can consume; served time and randomness on the log | Record once, replay a hundred times, equal state digests; a test that fails at random on a plain guest gives one answer on every replay |
| 16 | The world's snapshot and live input log go to a standby; a commit mark on every outgoing ring so a reply leaves only after the standby holds its inputs; on loss of the primary the standby takes over at the exact point the log reached; snapshots ship to the store in increments; a dead primary is fenced so it cannot return; warm standby by default, hot standby optional. Placement and rerouting belong to Realm. Single-processor worlds first | Power off one host mid-stream: the world resumes on the other, the client sees no reset, state digests match and no acknowledged write is lost; the delay the commit mark adds to a reply is measured |
| 17 | Per-world schedulers under enforced budgets; a core grant that carries topology, with ring notices before cores are added or revoked; a per-thread shared page with extend, preempt-requested and blocked bits; timers through user interrupts with a ring wake fallback | A world scheduler beside the default on a tail-latency workload |
| 18 | Panic log to a reserved region; trace ring with a user-space exporter; per-world metrics; A/B boot; kernel-to-kernel boot | A crash log is read offline; an upgrade rolls back on a failed boot |
| 19 | ACPI through a user-space interpreter; driver worlds for USB, input, display, audio, Wi-Fi and Bluetooth over passthrough; hibernate | Realm boots on the owner's HP ENVY x360 (i7-10510U) with touchpad, display, storage, audio and networking through driver worlds |
| 20 | WizFS: copy-on-write B-tree, checksums, snapshots, on the object store | Crash injection and a repair tool (D-08) |

## 5. Hardware the design assumes

FRED entry where the processor offers it, legacy entry otherwise. User
interrupts (Intel) for ring completions, with a ring wake fallback for every
other processor. LASS assumed on. Supervisor protection keys are not relied
on. AMX state is lazy and per thread. Scalable IOV maps one PASID per driver
shard when a NIC offers it. Five-level paging allows sparse arenas. x2APIC
remains required (D-33). The test laptop has neither user interrupts nor
protection keys: every mechanism is developed against QEMU and the
fallbacks, and the speed checks that need those features wait for hardware
chosen later.

## 6. Research tracks

- **Worlds sharing a process.** Worlds of one owner may share a process; a
  world from a different owner always gets its own hardened process. Each
  shared world is a domain: a kernel object inside the process with a
  memory protection key, its own handle table, arena, quota and kill, and
  an effect row checked by the compiler. It is entered only through a
  kernel gate, and every call resolves handles in the current world's
  table. The kernel
  checks the world's rights, not the process's, on every copy in and out; no
  register-restoring return path can change a world's key; no cross-world
  debugging. The study starts after Phase 8 and ends with a decision. Its
  first result is the attack suite: the known confused-deputy attacks on
  protection keys and the remote timing attacks on co-located tenants, run
  against the design. Its deciding measurement is ten thousand worlds as
  processes against one hundred processes of one hundred worlds, on memory
  and switch cost. No feature is called tenant isolation unless it is the
  process boundary or provably equal to it.
- **One address space for the machine.** Two domains share a ring over one
  object; if the saving over separate address spaces is under twofold on a
  ten gigabyte per second stream, the idea is retired.
- **Replay across processors.** Recorded worlds that share memory across
  processors, through a recorded schedule or recorded lock order.
- **A verified confidential-VM host.** After Phase 12, count exits per ten
  gigabytes of virtio traffic against KVM.
- **Per-tenant hardware queues** behind Scalable IOV hardware.

## 7. Where the code stands

What exists today that each phase must replace or build on; `docs/architecture.md` has the detail.

Phase 7:
- A process is a thread slot in global by-slot arrays (`kernel/src/user.rs`,
  `kernel/src/user/reap.rs`, `kernel/src/user/construction.rs`), with no parent, exit status or
  child observation. Phase 7 needs a counted process object owned by its world.
- Kernel memory comes from one heap that never shrinks and one handle arena behind one lock
  (`wost/src/mm/heap.rs`, `kernel/src/handle.rs`), so handle values reveal the shared arena's
  allocation order across processes. Worlds need a handle table per process carved from their
  arena (`wost/src/mm/pool.rs`), and user page tables taken as arena table frames.
- Frame ownership lives in a page-table software bit, nothing changes a permission or unmaps a
  shared mapping, and spawn takes each segment as one contiguous run with no quota
  (`wost/src/arch/x86_64/paging.rs`, `kernel/src/user/load.rs`). Memory objects need objects that
  own their frames, a protect primitive, a shared unmap and accounting that cannot starve a
  sibling.
- `DmaSegment::from` wraps a `Segment` without zeroing it, and each caller zeroes first
  (`wost/src/mm/dma.rs`). Memory a device reaches needs the same zeroing conversion as a `USegment`.
- The kernel parses ELF and loads one borrowed Limine module (`kernel/src/user/load.rs`,
  `wost/src/boot/limine_glue.rs`). The user-space loader replaces it and needs every module as owned
  frames; the kernel keeps its mapper checks.
- Threads are capped by a 32-slot task table and a 16-slot stack arena, and one `Binding` serves
  one thread (`kernel/src/thread.rs`, `wost/src/arch/x86_64/task.rs`). Processes need many threads
  per root.
- Each process has one fixed ring, timers expire only on kernel entry, and the time page is global
  (`kernel/src/queue.rs`, `kernel/src/timepage.rs`): every process reads the one root world clock.
  Channels need their own transport, and time needs a clock per world, the recorded mode and the raw
  time right.
- The ring 3 witness is hand-encoded machine code that restates `libs/native-abi` as literals
  (`xtask/src/image/program.rs`). Phase 7 needs witness programs built by cargo that import the ABI
  from `libs/`, and records of how each program ended. No ring 3 program faults on a kernel address to witness isolation,
  issues `RingWait` or `ThreadBase`, or executes `syscall`, so those witnesses must cover all
  three.
- Collection searches zombies from slot 0, so one whose reclaim is refused blocks every later
  one, and a refused loader rollback drops its ledger, so spawn cleanup then panics
  (`kernel/src/user/reap.rs`, `kernel/src/user/load.rs`). Neither refusal has a known trigger.
  Teardown of a process must never block another's and never panic.

Phase 8:
- Every piece of processor state is a single global, there is no per-CPU base, and the GDT, TSS,
  fault stacks and paired-root entry route are one static each that init loads once (`percpu.rs`,
  `interrupt.rs`, `task.rs`, `segment.rs` and `kpti.rs` in `wost/src/arch/x86_64/`). Phase 8 needs a
  `GS`-based per-CPU block and per-processor tables.
- Stack reuse is safe only on one processor: `exit` frees a slot still running on its stack
  (`kernel/src/thread.rs`). Phase 8 needs an on-CPU flag the next thread clears.
- There is one run queue and one idle thread, and one PIT watchdog judges one APIC window for a
  global clock published with relaxed stores (`kernel/src/thread.rs`, `kernel/src/time.rs`). Phase 8
  needs per-CPU queues and timers, one clock owner and release and acquire publication.
- Lock order is a convention: `lock_leaf` takes no token, and leaf locks cover whole-bus PCI walks
  (`wost/src/sync.rs`, `wost/src/arch/x86_64/pci.rs`). Fine-grained
  locking needs a checked order, and no caller-supplied work may run under a leaf guard.
- There is no TLB shootdown: invalidation and "is this root loaded" ask only this CPU, and the
  user-copy page walk can go stale under a concurrent unmap (`wost/src/arch/x86_64/paging/walk.rs`,
  `wost/src/arch/x86_64/usercopy.rs`).
- The local APIC has no second `init` and no IPI, and a panic stops only its own CPU
  (`wost/src/arch/x86_64/apic.rs`, `wost/src/panic.rs`). The QEMU machine has one CPU (`xtask/src/qemu.rs`).

Phase 9:
- One VT-d unit, one domain and one function (`wost/src/arch/x86_64/vtd.rs`, `kernel/src/iommu.rs`).
  Phase 9 needs a unit per DRHD and a domain per driver process.
- Isolating one function claims every sibling behind the unit, so one bus-mastering driver exists
  per machine and the inventory stops at 64 (`kernel/src/iommu.rs`). The empty root already denies
  siblings, so those claims go.
- Interrupt remapping is deny-all, with no entry allocator and queued invalidation torn down after
  one use, so every device is polled (`wost/src/arch/x86_64/vtd.rs`). Remapping stays enabled after
  queued invalidation is switched off, a state the VT-d specification does not describe.
  Per-driver vectors need a sized table and per-device entries.
- The second-level table arena never returns a table: an unmap clears the leaf and keeps every
  table it carved (`wost/src/arch/x86_64/vtd.rs`).
- PCI has no capability walk, MSI-X or ECAM, identity is the CF8 word, and `admit_bus_master`
  accepts only an NVMe proof (`libs/pci-config`, `wost/src/arch/x86_64/pci.rs`).
- Claims are linear tokens never dropped, and uncertain outcomes park in write-only statics
  (`kernel/src/pci.rs`, `kernel/src/iommu.rs`, `kernel/src/nvme.rs`). Restartable drivers need a
  revoke that cannot be refused.
- NVMe runs one command at a time with no reset path, maps each buffer per command with a global
  invalidation and holds the IOMMU lock across the wait (`kernel/src/nvme.rs`,
  `kernel/src/nvme/io.rs`). It maps the buffer before checking that a command may start and
  ignores a failed unmap on that path. A fenced driver needs per-core queues, long-lived mappings and a
  bring-up a reset can rerun.

Phase 10:
- The NVMe path is read-only: no Identify Controller, PRP lists, Write, Flush, shutdown
  notification, status decoding or 4096-byte blocks (`libs/nvme`, `kernel/src/nvme/io.rs`).
- The kernel parses FAT32 one sector per command at a fixed sector 2048 with no partition table, and
  walks a file's chain from its first cluster on every read (`libs/fat32`, `kernel/src/fs.rs`), so
  a program that picks a large `Read` offset holds the one worker for three device reads a cluster. The
  object store needs a partition table, a bounded block-range capability and extents over buffers
  mapped once.
- The VFS, tmpfs, devfs and file worker run in the kernel (`kernel/src/vfs.rs`,
  `kernel/src/queue/work.rs`). They are replaced, not extended.
- One global eight-slot file-work queue serves every program, so one program can fill it and
  starve the others (`kernel/src/queue/work.rs`).

Later phases:
- The platform is two hypervisors gated by a private `WZDH` table and their CPUID identity, with one
  DRHD, no RMRR and two VMware devices admitted by id (`kernel/src/iommu.rs`,
  `libs/pci-config/src/fence.rs`). Storage requires VT-d, AMD-Vi has no backend, and devices see
  physical addresses. Real hardware needs rules by device class.
- Without enhanced IBRS nothing mitigates branch-target injection from ring 3, and there is no BHI
  or SSBD handling (`libs/x86-cpuid`).
- `qualify-vmware` cannot tell a copied transcript from a fresh one; the guest gets no challenge.
- The host tool's QEMU boots are verified on Linux; on the Windows host it has only been compiled
  and unit-tested.
- A clock rated from CPUID leaf 0x15 has no independent check: the APIC timer is measured against
  the same counter and no watchdog runs (`kernel/src/time.rs`). No supported machine takes that
  path yet. Where the PIT counts, one PIT window must cross-check the rate before it is trusted.
- The topology snapshot honours the multifunction bit, so a device that answers at functions 1 to
  7 while function 0 claims one function is absent from the DMAR proof (`kernel/src/pci.rs`). The
  entry fence still clears its bus mastering and the empty VT-d root denies it.
- The toolchain and the `limine` 0.5.0 pin change only when a concrete need forces it.

## 8. Open decisions

| Decision | Decided when | Input |
|---|---|---|
| When worlds carry their own stack over their own queue | After Phase 13's counts | Per-packet entries and copies against the stack service; a durable world needs its connection state in its own memory |
| GPU device for Phase 19 | Before display work | VMware's SVGA and a driver world against the laptop's Gen 9.5 |
| Cloud NICs (virtio-net, ENA, gVNIC) | At a named deployment | That deployment |
| Ring version 2: unsolicited records for signals and scheduler notices (D-80, D-181) | At Phase 17 | Coalescing and overflow rules |
| A bounded runner for static Linux binaries, at most about eighty system calls, with fork, control groups, ptrace and io_uring never offered | Late, after the planned phases | Phase 12's clone measurements: whether a cloned box is too heavy for small static servers |
| Hardware for the user-interrupt and protection-key checks | When a phase needs them | Cost at that time |

## 9. What stays out

Hard real-time. A second architecture. Any compiler, parser or language
runtime inside the kernel beyond the extension checker and emitter. Full
Linux compatibility. Device access without an isolation domain. Dual-mapped
executable memory. Native drivers outside the speed path. Resource
disaggregation, far memory and pointer-capability hardware until the
hardware ships.
