# 002 - Native architecture (2026)

**Ownership after D-366 (2026-10-07).** Bedrock owns the kernel mechanisms
and ABI described here. Product userland, graphics policy, compatibility ports
and application delivery belong to [Realm](https://github.com/wizards-ecosystem/wizards-realm).
The original phase numbers and dated observations remain historical; they do
not create a second current product backlog in Bedrock.

Decided 2026-07-18. This doc supersedes the "POSIX-ish core" framing in
earlier discussion: the native OS is **capability-based and async-first**;
POSIX exists only as a translation layer for ported software.
Revised same day after the deep-research pass: see `003-research-notes.md`
for evidence and citations behind the choices marked [cited].

## Design pillars

1. **No ambient object authority.** [cited] Every external kernel object (process,
   thread, memory object, channel, file, socket, framebuffer surface) is
   reached through a
   **handle** carrying **rights bits** (read, write, map, duplicate,
   transfer, ...). A process's external-object authority is exactly its handle
   table. There are no global object namespaces reachable without a handle. No
   current handle-less call creates external object authority. Any future call
   that does must be governed by an explicit per-process **policy layer**; none
   is implemented for today's enumerated ring/time/control facilities. Handles
   are process-local integers. Duplicate attenuates rights today; future
   channel transfer must do the same. Handle-producing operations such as
   OpenAt derive authority from a handle already held.

   Delegation is stated as an invariant a check can hold us to, rather than
   as a claim about mechanisms: **a process receives exactly one handle at
   spawn: its opaque bootstrap-channel identity. Later authority is derived
   by a handle-authorized ring operation today and may arrive through a future
   channel operation.** Nothing can mint ambient object authority. D-151
   amended the original channel-only provenance once completions could carry
   handles; D-248 removed the uncalled channel mechanics without weakening the
   birth count.
   *Anchors: Zircon handles (verified), seL4 capabilities, and Capsicum's
   namespace confinement and per-object rights. Capsicum is not evidence for
   spawn-only: capability mode survives `fork` and `pdfork`.*

2. **Async-first I/O.** [cited] The native data-path ABI is **submission /
   completion queues** shared with the kernel (io_uring lineage): userspace
   posts operations tagged with handles; the kernel completes them out of
   order. The current synchronous control plane has four calls: retire the
   caller, enter or wait on its ring, and set its thread base. Duplicate is a
   ring operation; channel traffic is planned there too; user mapping still
   has no chosen execution contract (D-181). The large win comes
   from making the caller structurally asynchronous; batching improves both
   direct trap cost and indirect cache cost. Avoiding the trap entirely is a
   smaller additional gain and can cost a dedicated core (`010` section 2).
   Research-driven rules: **ring structures carry explicit version fields**
   (IoRing's improvement over io_uring); the opcode surface stays minimal
   with no privileged opcodes (io_uring's exploit record, which Google
   kernelCTF disables, is a warning about surface growth); every
   read from shared rings is defensively validated (TOCTOU).
   *Anchors: Linux io_uring, Windows IoRing, FlexSC (OSDI'10), Zircon
   ports (typed packets unifying all event sources: our completion
   path's model).*

3. **Spawn, not fork.** Native process creation targets
   `spawn(executable_handle) -> (process_handle, bootstrap_channel)`: the held
   executable authority authorizes the launch and both returned capabilities,
   while the kernel starts the child with exactly one channel endpoint.
   Arguments and grants are bootstrap messages (D-111, D-151, D-180). No
   native `fork()` (*"A fork() in the road"*, HotOS '19; Fuchsia). The POSIX
   layer emulates fork for ports that cannot live without it once Phase 9
   builds a gated compatibility clone; no such kernel primitive exists yet.

4. **Lightweight everywhere.** Tickless kernel (no idle timer ticks),
   EEVDF-inspired virtual-deadline scheduler, small static binaries,
   no daemons-by-default. Measure size and wakeups as features.

5. **Framekernel discipline.** [cited] (Adopted from Asterinas, USENIX ATC'25.)
   The kernel is one address space with monolithic performance, but ALL
   `unsafe` Rust is confined to a small privileged framework crate (`wost`,
   Bedrock's OSTD analogue: boot, paging, MMIO, port I/O,
   DMA, context switch, interrupt entry). Every other kernel crate
   (scheduler, VFS, WizFS, network, drivers) is `#![forbid(unsafe_code)]`
   safe Rust against `wost`'s safe abstractions. The containment property is
   verified here daily, and D-13 is Provisional because the paper's broader
   measurements have no independent replication (D-71, `010` section 1). Its ~14%
   TCB ratio is no longer used as evidence: absolute unsafe lines are the
   relevant audit cost, and only single-core syscall dispatch is a defensible
   performance summary. The authors' design lesson still stands: safe MMIO/DMA
   abstractions must be designed in from day one, not retrofitted.

   *Implemented 2026-08-12 (D-21). One amendment learned in the doing: the
   kernel **binary** carries `#![deny(unsafe_code)]`, not `forbid`, because
   `wost::entry!` expands its boot glue there and `forbid` cannot be lifted by
   an `allow` even inside a macro expansion. Library crates, which have no entry
   glue, keep `forbid`: `libs/pixel` already does.*

6. **Fault containment by supervision, not unwinding.** [cited] Theseus's
   fault-injection data (69% recovery; failures dominated by Rust/LLVM's
   missing async unwinding) shows panic-unwinding is not a containment
   mechanism. Services are designed restartable under a supervisor;
   `catch_unwind` is diagnostics only. Hardware isolation remains the
   boundary for all user code; language safety is a kernel-internal tool.

## Target kernel object model

| Object | Purpose |
|---|---|
| `Process` / `Thread` | execution; threads belong to a process's address space |
| `MemoryObject` | pageable memory (VMO-style); mappable, shareable, the currency of zero-copy I/O and compositor buffers |
| `Channel` | bidirectional message pipe carrying bytes + handles (the IPC primitive) |
| `Event` / `EventPair` | signaling, lifecycle notification |
| `Queue` | a submission/completion ring bound to the kernel |
| `File` / `Dir` | VFS objects (WizFS behind them) |
| `Socket` | network endpoints (smoltcp behind them) |

This is a target model, not an implementation inventory. The current
handle-object tags are Channel, File and Directory; `Channel` is only an
opaque, counted bootstrap identity, with no peer or message semantics (D-248).
The ring is an enumerated ambient facility. Process, Thread, MemoryObject,
Event, EventPair, Socket and a handle-bound Queue wait for their callers and
decisions. Monolithic-modular still holds: these belong in the kernel when
implemented; capability discipline applies at the ABI boundary, not via
userspace servers.

## Security baseline

On from the start:

- W^X for all mappings (no page ever writable+executable), NX enforced,
  live since Phase 3
- SMEP + SMAP + UMIP enabled in CR4 during early boot, live since Phase 2
- Zero-on-allocate pages (Phase 3); **guard pages under every stack that may
  run untrusted work**: thread stacks (Phase 4, D-52), the double-fault stack
  (D-89), and the boot thread's, which could not be guarded where the
  bootloader left it and so was left instead (D-92). If that final allocation
  refuses, diagnostic kernel service continues on the legacy stack but ring 3
  stays closed (D-193). Each boundary has a check that goes red without it

Deferred, each with its cost named where it was deferred:

- **KASLR** via Limine (`kaslr: yes`) waits on the hardcoded load address in
  `linker.ld`; deferred to Phase 12 with its interest recorded (D-46). It is
  the only D-12 item deferred as far as Phase 12, and it is *not* "on from the
  start". The stack exceptions this list used to carry are closed for every
  untrusted boundary (D-89, D-92, D-193).
- Intel CET shadow stacks, signed boot chain, measured boot. CET is the one
  that constrained earlier work: a shadow stack is a second region per thread,
  and the Phase 5 stack allocator must not foreclose it (`010` section 10).
  **Answered in `011` slice 3, and answered by *not* building it:**
  `libs/stack-layout` computes every address from `(base, stack_len,
  guard_len, slots)`, so a second region is a field on `Slot` and a term in the
  stride, in a host-tested crate with no hardware in it. An unused slot built
  today would be the speculative API `005` forbids; the arithmetic being
  parameterised is the whole of what was owed.

## Filesystem: WizFS is CoW

Copy-on-write B-tree filesystem, ZFS/btrfs/bcachefs lineage:

- never overwrite live blocks -> crash consistency by construction
- checksums on all data and metadata (detect bit rot, verify on read)
- cheap O(1) snapshots; zstd compression as a later flag
- designed for the VFS handle model (a `File` handle is a capability)

Research-driven design rules [cited] (bcachefs/btrfs/BetrFS evidence,
`003-research-notes.md`):

- **snapshots as key versioning** (bcachefs): keys are
  (inode, offset, snapshot_id); snapshot creation copies nothing,
  avoids btrfs's clone-the-whole-tree scalability trap
- **checksums live in the btree beside the pointers**, never co-located
  with data (co-located checksums cannot detect lost writes/stale reads)
- **journaled btree updates** with one totally ordered journal -> prefix
  consistency after crash; big log-structured nodes, own node cache
- **write-optimized upserts internally**, without making blind upserts an ABI
  rule: BetrFS showed the cache can satisfy POSIX preconditions without a disk
  read, so the technique is ABI-independent (D-73)
- no parity RAID in the initial design: correct parity needs buckets, copying
  collection, and backpointers first; RAID-Z itself does not have the classic
  write hole (D-73). Scrub/repair machinery remains part of the design

The format will live in `libs/wizfs-format` (no_std, host-testable + fuzzable)
when Phase 13 starts it; that crate is intentionally absent today.

## App formats

- **ELF64** native containers. First-party system and application source is
  Wizzy under D-183; a narrow Rust runtime/ABI witness or a later Wizzy AOT
  image may each be ELF. A witness is not a fallback native userland. Ported C
  reaches the native ABI through mlibc rather than defining it.
- **Core WebAssembly + `wasip1`** as a first-class citizen: WASI's authority
  model aligns with handle grants, while a WASI descriptor adapter owns
  numbering, preopens and cursor state over the cursorless native operations
  (D-181). A module compiled for this OS survives an architecture port. Both
  reasons are delivered by `wasip1`. Runtime: a wasmi-class `no_std`
  interpreter in user mode, pinned when Phase 7 starts rather than named here;
  WASI host bindings implemented by us directly over native handles
  (wasmtime's host is tokio/std-coupled and does not fit this target; its
  cap-std/WasiCtx pattern is the shape to follow).

  **The Component Model is a watch item, not a target** (D-72). It is still a
  Phase 1 proposal, its own 1.0 roadmap schedules an ABI change and lists
  "easier implementation" as open work, wasmi has no support and no plans, and
  the published component ecosystem is ten packages. Re-evaluate when it
  reaches Phase 4, or when an interpreter ships support. `010` section 6 has the
  evidence.

## Wizzy-native userland and compatibility gateways

D-183 fixes the implementation boundary: the kernel and its in-kernel modules
stay Rust, while every feasible first-party target-side program, service,
library, shell, tool and application is Wizzy. "Native" means direct handles,
completions and bootstrap protocols; it does not decide whether Wizzy's first
execution engine is bytecode or AOT. Phase 7 product implementation is stopped
until `015-wizzy-userland.md`'s compiler-readiness gate opens.

D-184's `016-pre-wizzy-foundation.md` is the finite register of kernel,
qualification and handoff work allowed while that gate is closed. It does not
pull the product interfaces below into the pre-gate foundation.

Narrow audited runtime/ABI glue that Wizzy cannot yet express, host build and
test tooling, third-party libraries and ports, POSIX compatibility code and
foreign-format runtimes are implementation substrate or compatibility, not
alternate native userlands. An exception narrows as the language grows. A new
first-party Rust target component requires a decision naming the missing Wizzy
capability and its migration boundary.

D-116's emitted caller remains a loader/ABI witness. After the readiness gate,
the first compiled boundary program is Wizzy; a Rust ELF may validate a low-
level seam but does not grow into `init`, a shell, service or tool. The exact
gate, evidence and dependency order are in `015-wizzy-userland.md`.

POSIX remains a compatibility layer. Core Wasm + `wasip1` remains a first-class
application format under D-72 rather than becoming the native process model or
substituting for the Wizzy-native path. First-party modules may be produced by
Wizzy once its backend supports the format; foreign modules remain a deliberate
application/compatibility case.

### POSIX translation layer (the ports gateway)

Phase 9's target is mlibc sysdeps over the native ABI in a userspace library:
POSIX descriptors own compatibility state over handles, blocking calls wrap
queue submissions and waits, and a future fork shim uses a gated compat clone.
That is the gateway intended to run CPython, links and NetSurf; none runs here
yet, and native software will not touch the layer.

**It was shaped early; its first real port waits for its compatibility phase**
(D-74, D-114, D-180). The stub-heavy sysdep was written as soon as Phase 5
could run two processes, not to run real software, but to find out whether a
POSIX libc could sit on this ABI while the ABI was still cheap to change.
Phase 5 slice 10 did that work against a toolchain wall recorded rather than
worked around: the sysdeps are analysed, not compiled, and
`fixtures/abi-pressure/README.md` states what that cost.

**The surface was measured, so it is not costed at zero.** At D-74's
unrecorded upstream snapshot, mlibc exposed 296 sysdep tags, called sixteen
mandatory and described those as enough for a basic static hello world; the
real ports inspected implemented 147 to 202. D-181 forbids treating those
unversioned figures as a current API. A real Phase 9 port pins a release before
adopting its `abis/linux` userspace-visible ABI.

Two things this layer must answer that the native ABI would otherwise never
raise, and both were cheaper to expose before Phase 5 froze the queue. D-80
settled that signals share the completion plane and do not push kernel frames;
the versioned event record, dispatch/correlation and interruptible libc wait
remain Phase 9 work (D-181). Whether userland is static-only or gets a dynamic
linker is also open there (CPython needs `dlopen` and `-export-dynamic`). POSIX
uid/gid, umask, controlling-TTY, hard-link and signal-mask conventions require
explicit compatibility state, routing or policy rather than ambient native
defaults. Plan 9's APE and Fuchsia POSIX Lite show their cost, not a proof that
they cannot be represented. `010` section 5 has the evidence and numbers.

## Graphics direction

Compositor speaks `Channel` + shared `MemoryObject` buffers (Wayland's
model, our protocol). Rendering is GPU-first (wgpu/vello-style compute 2D)
targeting virtio-gpu in VMs, with a dumb-framebuffer fallback.

The shell model, the compositor-drawn chrome and the toolkit's token contract
are fixed in `019-userland.md` and `020-style-guide.md` (D-318, D-321);
the CPU rendering fallback covers the toolkit as well as the framebuffer.

## Open questions (future docs)

Still open:

- **WizFS on-disk format specifics** (before its own phase, D-73)
- **Compat-fork mechanics for mlibc** (before Phase 9)
- **How a program receives its first directory handle.** Phase 6 built
  `OpenAt` and a program cannot reach it: `spawn` gives exactly one handle
  (the opaque bootstrap-channel identity, D-111 and D-248), and no ring-3
  channel-receive operation exists yet. A second handle at spawn would break
  the invariant that makes pillar 1 checkable, so this waits for a concrete
  channel protocol rather than for a convenience (D-173).

Closed since this list was written:

- *Whether a POSIX libc can sit on this ABI*: pressure-tested by D-114 rather
  than deferred. D-181 corrected the result: one label could not say both what
  the uncompiled C++ body does and what platform work remains. The exact
  per-function BODY/REMAINDER register is in `fixtures/abi-pressure/README.md`. At every
  commit the BODY claims are checked against discovered, comment-free source;
  REMAINDER remains reviewed platform judgment with targeted evidence checks.

  The retired one-axis count is not repeated here. This paragraph once said
  "three map, twelve need a door" for a day after D-118 had made its own
  arithmetic four and eleven; D-181 then showed that the categories themselves
  were false for log, write, seek and futex. A checked row is stronger than a
  duplicated tally (D-94, D-124).

  `fixtures/abi-pressure/README.md` also has the four things writing the sysdeps surfaced
  that the native ABI would never have raised.

- *Handle table representation & rights taxonomy*: the taxonomy half by
  `010` section 3, the layout and lifetime rules by **D-78** and
  `011-userspace.md`'s handle-table section: one global arena, generation-tagged
  arithmetic, no lowest-free promise, a refcounted table, and no revocation.
- *Queue memory layout & wakeup semantics*: answered by **D-79** and
  `011`'s queue section: one terminal completion per operation, kernel-owned
  buffers, reservation-based backpressure, copy-out-then-validate, and ports
  unified into the completion queue.
- *Which plane carries a signal*, answered by **D-80**: it must share the
  completion plane, with no kernel-written signal frame. D-181 records the
  unfinished half: version 1 has no unsolicited-event record, so dispatch,
  correlation and interruptible libc-wait semantics remain Phase 9 work.
- *Kernel-memory accounting, untyped-retype or not*, answered by **D-44**:
  rejected as an accounting mechanism, adopted as a type discipline.
- *CHERI/CET/PKS hardware security pass*: closed by `008` section 6, and updated by
  `010` section 10 with FRED, LASS and the cancellation of x86S.
- *Re-verify `003`'s Tier-2 claims*: done in `010`; most confirmed, and the
  ones that were not are recorded there.
- *Framekernel SMP scalability*: still unquantified by anyone, and now known
  to be worse than that: the authors' own fix became a separate paper that
  changes the design (`010` section 1). This is why D-13 is Provisional.
