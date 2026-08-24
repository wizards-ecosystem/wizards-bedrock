# 002 — Native Architecture (state of the art, 2026)

Decided 2026-07-18. This doc supersedes the "POSIX-ish core" framing in
earlier discussion: the native OS is **capability-based and async-first**;
POSIX exists only as a translation layer for ported software.
Revised same day after the deep-research pass — see `003-research-notes.md`
for evidence and citations behind the choices marked ★.

## Design pillars

1. **No ambient authority.** ★ Every kernel object (process, thread, memory
   object, channel, file, socket, framebuffer surface) is reached through a
   **handle** carrying **rights bits** (read, write, map, duplicate,
   transfer, …). A process's power is exactly its handle table. There are
   no global namespaces reachable without a handle. Handle-less syscalls
   (object creation, ring setup) are governed by an explicit per-process
   **policy layer** — the escape hatch Zircon implements as Job policy;
   verified as the deployed Fuchsia design. Handles are process-local
   integers; rights attenuation on duplicate, replace **and send**.

   Delegation is stated as an invariant a check can hold us to, rather than
   as a claim about mechanisms: **a process receives exactly one handle at
   spawn — its bootstrap channel — and every later handle arrived through a
   channel.** (The earlier wording, "channels are the sole delegation
   mechanism", was false even of Zircon, which also passes handles through
   `zx_process_start`. See `010` §3.)
   *Anchors: Zircon handles (verified), seL4 capabilities, capsicum
   (whose capability mode independently forced fork removal — convergent
   evidence for pillar 3).*

2. **Async-first I/O.** ★ The native data-path ABI is **submission /
   completion queues** shared with the kernel (io_uring lineage): userspace
   posts operations tagged with handles; the kernel completes them out of
   order; a small set of synchronous syscalls covers setup, handle ops,
   memory mapping, and blocking waits (doorbell/`enter`). The win comes
   from batching + cache locality (FlexSC), not from avoiding the trap.
   Research-driven rules: **ring structures carry explicit version fields**
   (IoRing's improvement over io_uring); the opcode surface stays minimal
   with no privileged opcodes (io_uring's exploit record — Google
   kernelCTF disables it — is a warning about surface growth); every
   read from shared rings is defensively validated (TOCTOU).
   *Anchors: Linux io_uring, Windows IoRing, FlexSC (OSDI'10), Zircon
   ports (typed packets unifying all event sources — our completion
   path's model).*

3. **Spawn, not fork.** Native process creation is
   `spawn(image, initial_handles, args)` — the parent explicitly grants the
   child its starting capability set. No native `fork()`
   (*"A fork() in the road"*, HotOS '19; Fuchsia). The POSIX layer emulates
   fork for ports that cannot live without it (kernel provides an
   address-space-clone primitive gated as a compat capability).

4. **Lightweight everywhere.** Tickless kernel (no idle timer ticks),
   EEVDF-inspired virtual-deadline scheduler, small static binaries,
   no daemons-by-default. Measure size and wakeups as features.

5. **Framekernel discipline.** ★ (Adopted from Asterinas, USENIX ATC'25 —
   the strongest verified result of the research pass.) The kernel is one
   address space with monolithic performance, but ALL `unsafe` Rust is
   confined to a small privileged framework crate (`wost` — Wizard OS
   Standard, our OSTD analogue: boot, paging, MMIO, port I/O, DMA,
   context switch, interrupt entry). Every other kernel crate — scheduler,
   VFS, WizFS, network, drivers — is `#![forbid(unsafe_code)]` safe Rust
   against `wost`'s safe abstractions. Asterinas proved this yields a
   microkernel-sized memory-safety TCB (~14%) at Linux-parity single-core
   performance, and its authors' verified lesson is that safe MMIO/DMA
   abstractions must be designed in from day one, not retrofitted.

   *Implemented 2026-08-12 (D-21). One amendment learned in the doing: the
   kernel **binary** carries `#![deny(unsafe_code)]`, not `forbid`, because
   `wost::entry!` expands its boot glue there and `forbid` cannot be lifted by
   an `allow` even inside a macro expansion. Library crates, which have no entry
   glue, keep `forbid` — `libs/pixel` already does.*

6. **Fault containment by supervision, not unwinding.** ★ Theseus's
   fault-injection data (69% recovery; failures dominated by Rust/LLVM's
   missing async unwinding) shows panic-unwinding is not a containment
   mechanism. Services are designed restartable under a supervisor;
   `catch_unwind` is diagnostics only. Hardware isolation remains the
   boundary for all user code; language safety is a kernel-internal tool.

## Kernel object model (initial set)

| Object | Purpose |
|---|---|
| `Process` / `Thread` | execution; threads belong to a process's address space |
| `MemoryObject` | pageable memory (VMO-style); mappable, shareable, the currency of zero-copy I/O and compositor buffers |
| `Channel` | bidirectional message pipe carrying bytes + handles (the IPC primitive) |
| `Event` / `EventPair` | signaling, lifecycle notification |
| `Queue` | a submission/completion ring bound to the kernel |
| `File` / `Dir` | VFS objects (WizFS behind them) |
| `Socket` | network endpoints (smoltcp behind them) |

Monolithic-modular still holds: these are in-kernel objects; the capability
discipline applies at the ABI boundary, not via userspace servers.

## Security baseline

On from the start, and all of it live since Phase 3:

- W^X for all mappings (no page ever writable+executable), NX enforced
- SMEP + SMAP + UMIP enabled in CR4 during early boot
- Guard pages under every stack; zero-on-allocate pages

Deferred, each with its cost named where it was deferred:

- **KASLR** via Limine (`kaslr: yes`) — waits on the hardcoded load address in
  `linker.ld`; deferred to Phase 12 with its interest recorded (D-46). It is
  the last of D-12's baseline still owed, and it is *not* "on from the start".
- Intel CET shadow stacks, signed boot chain, measured boot. CET is the one
  that constrains earlier work: a shadow stack is a second region per thread,
  so the Phase 5 stack allocator should not foreclose it (`010` §10).

## Filesystem: WizFS is CoW

Copy-on-write B-tree filesystem, ZFS/btrfs/bcachefs lineage:

- never overwrite live blocks → crash consistency by construction
- checksums on all data and metadata (detect bit rot, verify on read)
- cheap O(1) snapshots; zstd compression as a later flag
- designed for the VFS handle model (a `File` handle is a capability)

Research-driven design rules ★ (bcachefs/btrfs/BetrFS evidence,
`003-research-notes.md`):

- **snapshots as key versioning** (bcachefs): keys are
  (inode, offset, snapshot_id); snapshot creation copies nothing —
  avoids btrfs's clone-the-whole-tree scalability trap
- **checksums live in the btree beside the pointers**, never co-located
  with data (co-located checksums cannot detect lost writes/stale reads)
- **journaled btree updates** with one totally ordered journal → prefix
  consistency after crash; big log-structured nodes, own node cache
- **blind upserts in the native API**: POSIX preconditions force
  read-before-write; our handle ABI doesn't — keep the write-optimized
  fast path open (BetrFS lesson)
- no parity RAID, ever (write hole); scrub/repair machinery is part of
  the design, not an afterthought (ZFS lesson)

Format lives in `libs/wizfs-format` (no_std, host-testable + fuzzable).

## App formats

- **ELF64** native binaries (Rust userland, ported C via mlibc)
- **Core WebAssembly + `wasip1`** as a first-class citizen: WASI's capability
  model maps 1:1 onto our handles (a WASI descriptor *is* a handle grant), and
  a module compiled for this OS survives an architecture port. Both of those
  reasons are delivered by `wasip1`. Runtime: a wasmi-class `no_std`
  interpreter, pinned when Phase 7 starts rather than named here; WASI host
  bindings implemented by us directly over native handles (wasmtime's host is
  tokio/std-coupled and unusable in-kernel; its cap-std/WasiCtx pattern is the
  shape to follow).

  **The Component Model is a watch item, not a target** (D-72). It is still a
  Phase 1 proposal, its own 1.0 roadmap schedules an ABI change and lists
  "easier implementation" as open work, wasmi has no support and no plans, and
  the published component ecosystem is ten packages. Re-evaluate when it
  reaches Phase 4, or when an interpreter ships support. `010` §6 has the
  evidence.

## POSIX translation layer (the ports gateway)

mlibc sysdeps implemented over the native ABI in a userspace library:
POSIX fds map to handles, blocking calls wrap queue submissions + waits,
fork uses the gated compat clone. CPython, links, and NetSurf run on this;
native software never touches it.

**It arrives with the ABI, not after the userland** (D-74). A stub-heavy
sysdep is written as soon as Phase 5 can run two processes — not to run real
software, but to find out whether a POSIX libc can sit on this ABI while the
ABI is still cheap to change.

**The surface, measured, so it is not costed at zero.** mlibc defines 296
sysdep tags. Sixteen are mandatory, and its own porting book says those are
"enough for running a basic statically-linked hello world program"; real ports
implement 147 to 202. Every shipping port implements `Fork`, `Sigaction`,
`Ioctl`, `Termios`, `Poll` and `Waitpid`. Adopting mlibc also means adopting
`abis/linux`, so the *userspace-visible* ABI becomes Linux's — pinned to a
release tag, never `master`.

Two things this layer must answer that the native ABI would otherwise never
raise, and both are cheaper to answer before Phase 5 freezes the queue:
**how a signal interrupts a blocking queue wait**, and whether userland is
static-only or gets a dynamic linker (CPython needs `dlopen` and
`-export-dynamic`). The cluster POSIX assumes and a capability system does not
— uid/gid, umask, controlling tty, hard links, signal masks — is the same
cluster Plan 9's APE could not implement and Fuchsia excluded from POSIX Lite.
`010` §5 has the evidence and the numbers.

## Graphics direction

Compositor speaks `Channel` + shared `MemoryObject` buffers (Wayland's
model, our protocol). Rendering is GPU-first (wgpu/vello-style compute 2D)
targeting virtio-gpu in VMs, with a dumb-framebuffer fallback.

## Open questions (future docs)

Still open:

- **Handle table representation & rights taxonomy** (doc before Phase 5). The
  taxonomy half is now answered — 16–24 rights in one `u32`, and a right must
  name a distinction the kernel makes about the object, never a syscall
  (`010` §3). What the doc still owes is the layout and the lifetime rules.
- **Queue memory layout & wakeup semantics** (doc before Phase 5); kernel
  executor follows waker discipline: no allocation in IRQ handlers,
  lost-wakeup re-check pattern, `Pin` for self-referential futures. `010` §2
  adds the austerity rules and the four hard problems this doc must answer.
- **How a signal interrupts a blocking queue wait** — the question that is
  hardest to answer late, and the reason mlibc moved forward (D-74).
- **WizFS on-disk format specifics** (before its own phase, D-73)
- **Compat-fork mechanics for mlibc** (before Phase 9)

Closed since this list was written:

- *Kernel-memory accounting, untyped-retype or not* — answered by **D-44**:
  rejected as an accounting mechanism, adopted as a type discipline.
- *CHERI/CET/PKS hardware security pass* — closed by `008` §6, and updated by
  `010` §10 with FRED, LASS and the cancellation of x86S.
- *Re-verify `003`'s Tier-2 claims* — done in `010`; most confirmed, and the
  ones that were not are recorded there.
- *Framekernel SMP scalability* — still unquantified by anyone, and now known
  to be worse than that: the authors' own fix became a separate paper that
  changes the design (`010` §1). This is why D-13 is Provisional.
