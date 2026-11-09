# 021 - The roadmap

This document orders Bedrock's kernel work from the foundation close onward
and fixes the design choices the order rests on. It replaces the phase list
in `000-vision.md` from Phase 7 on; Phases 0 to 6 keep their history. The
decision that records these choices follows in the decision log. Ownership
is unchanged (D-366): Bedrock owns kernel mechanisms, the native ABI,
drivers and qualification; Realm owns the operating environment; Ink owns
Wizzy.

Two rules of the previous plan end here. D-184's register in
`016-pre-wizzy-foundation.md` remains the record of the foundation close
(PW-006, PW-010), but a kernel mechanism no longer waits for a Wizzy caller.
Each phase below ships with its own witness programs, written in Rust and
retired when Realm's Wizzy programs replace them; they are checks, not a
userland, so D-183's rule that first-party product software is Wizzy stands.
Every ABI change is versioned (`002` pillar 2), so a witness never freezes an
interface.

## 1. Position

Bedrock is the small, provably isolating base under servers. A tenant runs
as a Bedrock process, made cheap enough that thousands fit on one machine,
or, when it needs Linux, as a Linux guest in a box Bedrock controls. Linux
is hosted, never imitated. On that base Bedrock offers four things a
general-purpose kernel cannot add cleanly:

1. **A tenant is a value.** A nested process group can be paused, copied and
   restored in under a millisecond, with its identity reset by the kernel.
2. **A device queue is a capability.** Network and storage data lands in the
   tenant's own memory behind the IOMMU, and completions reach it without a
   kernel entry where the processor allows.
3. **A tenant installs its own scheduler** for the cores and time its parent
   granted, and the kernel enforces that budget.
4. **A tenant can run deterministically and be replayed bit for bit**, because
   every input it receives arrives as a ring completion.

Wizzy is the policy and extension language and the control plane. The laptop
is the owner's showcase, with Linux in a box for the hardware and programs
it lacks. Nothing here claims a general-purpose consumer desktop.

## 2. Design choices fixed here

| Choice | Decision |
|---|---|
| Program loading | A user-space loader maps segments into a new address space and starts a thread. The kernel parses no executable format. Spawn takes a sealed, execute-bearing memory object (section 2, code loading) |
| IPC | The asynchronous ring (D-06) plus a synchronous call and reply path that switches directly from caller to server. Channels carry bytes and handles (D-110) |
| Grouping | Processes live in kernel-enforced groups that nest. Policy, limits and kill attach to a group |
| Kernel memory | Each group owns an arena of frames its parent donated at spawn or later. Every kernel object the group creates is carved from its arena by the kernel's own allocator; object sizes stay private; a dead group's arena is freed whole after the grace period. The kernel never allocates from a shared pool for a user request |
| Multi-core | Application processors start right after the process and IPC basics, with fine-grained locking from the first bring-up. This amends D-154's placement and D-70's big-lock first step |
| Drivers | One driver, two homes. A driver is safe Rust that reaches hardware only through tokens `wost` issues (D-13). It is sharded per core: one queue pair, one vector and one single-producer ring per core, with no shared mutable state on the data path. Storage, network, graphics and radio drivers run as fenced processes with their own address space, IOMMU domain and budget; small latency-sensitive drivers may run inside a budgeted kernel executor from the same source. Rings and buffers belong to the client, so a restarted driver is reconnected, not repaired. The intent log of submitted-but-incomplete operations is part of the ring contract, so storage restart is reset and resubmit |
| Network stack | A program on top of the kernel, never inside it. The kernel hands out device queues; a stack service holds the protocol core, which stays a platform-independent crate so a program can later carry its own stack over its own queue with no interface change. Programs see one socket interface whatever the home. Queues between driver, stack and program are batched shared memory, which is what sets the per-packet cost. This replaces D-20's in-kernel stack and keeps its driver order |
| Kernel extensions | Wizzy programs run in kernel hook points (driver ingress, ring filters, every rights derivation, group lifecycle, scheduling, tracepoints). The kernel holds only a type, effect and linearity checker for a small typed intermediate form and a tested emitter; no symbolic verifier and no compiler. eBPF bytecode is accepted through a second front end that lifts into the same form and fails closed |
| Tenancy | A cheap process per tenant is the product; nesting groups let a tenant spawn sub-tenants with fewer rights. Sub-process domains inside one process are a research track (section 6) |
| Code loading | Write, seal, execute (D-114 item 4 replaced): seal consumes the handle and returns a new one, there is no unseal and no dual mapping, code generation is a transferable quota object, executable placement is randomized, and seal refuses while any live write reference exists |
| Storage | A content-addressed object store on raw NVMe comes first and holds tenant snapshots and images. WizFS (D-73) is built on it later |
| Linux | Linux runs as a guest. A bounded user-space runner later translates a published list of about forty to eighty system calls for static server binaries; fork, control groups, ptrace and io_uring are never offered |
| Architecture | x86_64 only. The `arch/` seams stay so this remains a choice (amends D-02) |
| Hardware | Real hardware waits until the mechanisms exist. QEMU is the loop and VMware the milestone (D-18) through every phase before the laptop phase |

## 3. Phase order

```text
Foundation close (PW-006, PW-010)
  7  Processes, groups and arenas          witness: init, supervisor, echo
  8  Multi-core                            witness: the same on 2 and 4 CPUs
  9  Drivers as processes                  witness: virtio and NVMe fenced
 10  Storage write path and object store   witness: snapshot image round trip
 11  Tenants as values                     witness: clone 16, resume, diff
 12  Networking                            witness: echo over the fenced NIC
 13  Extensions, schedulers, replay        witness: filter, policy, replay
 14  Guests and the Linux door             witness: unmodified Linux boots
 15  Operability                           witness: crash log read offline
 16  The laptop showcase                   witness: Realm on the HP ENVY
 17  WizFS                                 witness: crash injection, repair
Research tracks run beside 9 onward (section 6).
```

Each phase closes with a QEMU self-test on 1, 2 and 4 processors from
Phase 8 on, a VMware boot (D-18), a persistent mutation per new check
(D-28), and a model-checked protocol where the phase adds concurrency
(D-112).

## 4. Mechanisms by phase

| Phase | Mechanism | Witness and proof |
|---|---|---|
| 7 | Groups with policy, limits and kill; per-group arenas; process object; spawn from a sealed memory object; exit status and child observation | A Rust supervisor spawns one child that exits 0 and one that faults, observes both, and a third that over-fills its arena is refused without touching its sibling |
| 7 | Channels with handle transfer; versioned bootstrap message; the call and reply path | An echo service answers through both paths; a handoff witness transfers every endpoint to a successor with no message lost or duplicated |
| 7 | Memory objects: map, unmap with a flush token, write-then-seal, execute right | Mutations: seal with a writable mapping, map executable without the right, mint the right without the capability |
| 7 | Directory enumeration; console and input device handles; user-space loader | The loader starts the supervisor from the image; `ls` over the ring |
| 8 | Application processors through Limine's request; per-processor stacks, tables, speculation controls and timer; per-processor run queues on `libs/eevdf`; TLB shootdown; PCID under paired roots (D-407) | TLA+ models of shootdown and the lost-wakeup re-check join `cargo xtask model-check`; a stress suite with a stated flake policy; no lock from Phase 7 is replaced by a global one |
| 9 | Driver tokens in `wost`; per-core shards; client-owned rings; one IOMMU domain per driver with static mappings; generation-tagged reconnect; the intent log | virtio-net and virtio-blk, then NVMe and e1000e, as fenced processes against the in-kernel executor on four processors: within ten percent at batch 32; kill the driver once a second under a write stream, recover in under 50 ms with no acknowledged write lost |
| 10 | NVMe write path; tmpfs mutation; the content-addressed object store with copy-on-write chunks | An image written, deduplicated and read back by digest; FAT32 stays read-only |
| 11 | Group snapshot to a handle; clone; resume; copy-on-write arenas; ring quiescence; kernel-reset clone identity | A four-process group with open rings is snapshotted, cloned sixteen times and resumed; each clone under a millisecond, resume from the store under ten |
| 12 | CSPRNG from `RDSEED`; NIC queues as capabilities; the stack service over the fenced e1000e with one socket interface; tenant fairness; MSI-X through remapping (D-372); user interrupts for completions where present, ring wake otherwise; vmxnet3 and cloud NICs behind named deployments | Echo through the stack service on QEMU and VMware; the handoff witness repeats with a listening socket; two hostile tenants share one NIC and neither reads the other's frames; kernel entries and copies per packet are counted and recorded |
| 13 | The typed intermediate form and its checker; hook points; eBPF front end; per-group schedulers under enforced budgets; deterministic groups with kernel-served time and a completion log | A packet filter and a rights policy in Wizzy beside their eBPF twins; a group scheduler beside the default on a tail-latency workload; record once, replay a hundred times, equal state digests |
| 14 | A virtual processor as a thread attribute and extended page tables as an address-space variant; virtio backends; the Linux door runner | An unmodified Linux guest boots on QEMU and VMware with virtio network and block; a static Linux binary runs as a Bedrock process through the door |
| 15 | Panic log to a reserved region; trace ring with a user-space exporter; per-group metrics; A/B boot; kernel-to-kernel boot | A crash log is read offline; an upgrade rolls back on a failed boot |
| 16 | xHCI, USB HID and storage; ACPI through a user-space interpreter; I2C HID input; Gen 9.5 display modesetting; HDA audio; a Linux driver guest for Wi-Fi and Bluetooth over passthrough; hibernate | Realm boots on the owner's HP ENVY x360 (i7-10510U) with touchpad, display, storage, audio and networking through the guest |
| 17 | WizFS: copy-on-write B-tree, checksums, snapshots, on the object store | Crash injection and a repair tool (D-73) |

## 5. Hardware the design assumes

FRED entry where the processor offers it, legacy entry otherwise. User
interrupts (Intel) for ring completions, with a ring wake fallback for every
other processor. LASS assumed on. Supervisor protection keys are not relied
on. AMX state is lazy and per thread. Scalable IOV maps one PASID per driver
shard when a NIC offers it. Five-level paging allows sparse arenas. x2APIC
remains required (`001`).

## 6. Research tracks

- **Sub-process domains.** A domain is a kernel object inside a process: a
  memory protection key, a capability set and an effect row checked by the
  compiler. The kernel checks the domain's rights, not the process's, on
  every copy in and out; no register-restoring return path can change a
  domain's key; no cross-domain debugging. The study starts after Phase 8
  and ends with a decision. Its first result is the attack suite: the
  known confused-deputy attacks on protection keys and the remote timing
  attacks on co-located tenants, run against the design. No feature is
  called tenant isolation unless it is the process boundary or provably
  equal to it.
- **One address space for the machine.** Two domains share a ring over one
  object; if the saving over separate address spaces is under twofold on a
  ten gigabyte per second stream, the idea is retired.
- **A verified confidential-VM host.** After Phase 14, count exits per ten
  gigabytes of virtio traffic against KVM, and write the capability core's
  invariants in Verus.
- **Per-tenant hardware queues** behind Scalable IOV hardware.

## 7. Open decisions

| Decision | Decided when | Input |
|---|---|---|
| When programs carry their own stack over their own queue | After Phase 12's counts | Per-packet entries and copies against the stack service |
| GPU device for Phase 16 | Before display work | VMware's SVGA against the laptop's Gen 9.5 |
| Cloud NICs (virtio-net, ENA, gVNIC) | At a named deployment | That deployment |
| Ring version 2: unsolicited records for signals and scheduler notices (D-80, D-181) | At Phase 13 | Coalescing and overflow rules |

## 8. What stays out

Hard real-time. A second architecture. Any compiler, parser or language
runtime inside the kernel beyond the extension checker and emitter. Full
Linux compatibility. Device access without an isolation domain. Dual-mapped
executable memory. Resource disaggregation and far memory until the hardware
ships. Wi-Fi as a native driver.
