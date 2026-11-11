# 003 — Deep Research: State-of-the-Art Kernel & OS Architecture (2026)

> **Historical research snapshot.** This is the evidence available at the
> founding design pass, not the current normative architecture. `010` re-checks
> it and corrects the framekernel metric and performance claim, the async-ABI
> rationale, Capsicum's rights count, the WASI target, and two WizFS rules.
> Current decisions live in `004-decision-log.md` and `002-architecture.md`.

Produced 2026-07-18 from 26 fetched sources. Of 25 claims put to adversarial
three-vote verification, 11 were confirmed, 0 refuted and 14 left unverified
because the verification run was interrupted; those are Tier 2 below. Findings
feed `002-architecture.md`; "Actions" lists the design changes adopted.

## Tier 1 — Verified findings (adversarial 3-vote confirmation)

### 1. Zircon's handle-centric authorization is a proven template (3-0)

Kernel objects perform **no security checks themselves**; all rights live on
per-process handles. Zircon is not fully handle-mediated: handle-less creation
calls (e.g. `zx_channel_create`) are governed by **Job policy**
(`zx_job_set_policy`).
→ *Adopt handle-attached rights and an explicit process/job policy layer for
handle-less operations.*
Sources: [fuchsia.dev kernel objects](https://fuchsia.dev/fuchsia-src/reference/kernel_objects/objects),
[kernel concepts](https://fuchsia.dev/fuchsia-src/concepts/kernel/concepts),
[handles](https://fuchsia.dev/fuchsia-src/concepts/kernel/handles)

### 2. The framekernel (Asterinas, USENIX ATC'25) is the validated Rust-kernel architecture (3-0 ×3)

One address space and plain function calls, but **all `unsafe` confined to a
small privileged framework (OSTD)**; every service is **100% safe Rust**.
Memory-safety TCB ≈ **14.0%** of the codebase vs Tock 43.8%, Theseus 62.4%,
RedLeaf 66.1% (2-1 verified; self-measured by the authors). 210+ Linux
syscalls at about Linux-parity single-core performance (LMbench geomean 1.08,
Nginx 1.17, Redis 1.31, SQLite 0.85), with caveats: single-core only, Linux
5.15, mitigations=off, smoltcp lacks congestion control. The authors' verified
lesson: **retrofitting safety onto legacy C (Rust-for-Linux) leaves unfixable
soundness holes; a clean-slate kernel must provide safe MMIO, port-I/O and DMA
abstractions so drivers can be 100% safe Rust.**
→ *Adopt the framekernel discipline, a hard privileged-crate boundary, rather
than scattering `unsafe` per subsystem.*
Sources: [ATC'25 paper](https://www.usenix.org/conference/atc25/presentation/peng-yuke),
[arXiv:2506.03876](https://arxiv.org/abs/2506.03876),
[;login: article](https://www.usenix.org/publications/loginonline/asterinas-rust-based-framekernel-reimagine-linux-2020s),
[OSTD book](https://asterinas.github.io/book/ostd/)

### 3. Theseus: language-based isolation works, panic-unwinding recovery does not (3-0 ×2)

Theseus (OSDI'20) runs everything at Ring 0 in one address space with Rust's
type system as the isolation, which proves language isolation viable for
in-kernel components. Its 800K-fault injection campaign recovered only **69%**
of manifested faults; about 76% of failures traced to Rust/LLVM's **missing
asynchronous unwinding**, still open in 2026.
→ *Treat `catch_unwind`-style recovery as best-effort only; contain faults with
supervised restartable components (as Hubris and Zircon do). Keep hardware
isolation for arbitrary user code; language isolation is a kernel-internal
tool.*
Sources: [Theseus OSDI'20 PDF](https://www.usenix.org/system/files/osdi20-boos.pdf),
[rust async-unwind issue](https://github.com/rust-lang/unsafe-code-guidelines/issues/211)

## Tier 2: sourced but unverified (re-verify before relying on specifics)

### Async ABI (FlexSC, io_uring, IoRing)

- FlexSC (OSDI'10): the raw mode switch is cheap (~150 cycles); the indirect
  cost (cache/TLB pollution, up to 65% IPC degradation) dominates. Async ABIs
  win via **batching, locality and core specialization**, not by avoiding the
  trap. [Soares & Stumm](https://www.usenix.org/legacy/event/osdi10/tech/full_papers/Soares.pdf)
- About 60% of Google kernelCTF exploit submissions targeted io_uring; Google
  **disables it** on hardened targets. → keep the queue opcode surface minimal,
  no privileged opcodes, and validate every shared-memory read defensively
  (TOCTOU). [kernelCTF rules](https://google.github.io/security-research/kernelctf/rules.html)
- Windows IoRing shipped **explicit structure versioning**; io_uring evolves by
  flags → version our ring structures from day one. Windows has the kernel map
  the rings, where Linux has userspace mmap them.
  [comparison](https://windows-internals.com/ioring-vs-io_uring-a-comparison-of-windows-and-linux-implementations/)
- Zircon's async model is **port-centric**: every event source arrives as a
  typed packet in one FIFO port; `zx_object_wait_async` is one-shot; ports
  service thread pools. A useful simplification for our completion path.

### Capability mechanics

- Zircon handle = (object ref, rights, owner process); values are
  process-local 32-bit ints; attenuation via `zx_handle_replace`;
  channels are the *sole* delegation mechanism.
- seL4: 10 object types; resource accounting by **retyping Untyped memory**
  (answers kernel-memory DoS); revocation is long-running and needs
  "incremental consistency" (abortable sub-operations). Claimed ~720-cycle IPC.
  [whitepaper](https://sel4.systems/About/seL4-whitepaper.pdf)
- Capsicum (later counted as 81 rights in `010` §3, per-logical-method):
  capability mode = **denying all global namespaces**, with process
  descriptors as handle-like references. Subsequent correction: capability
  mode is inherited across `fork` and `pdfork`; `pdfork` creates a child and
  returns a descriptor for it. Capsicum therefore supports confinement and
  process references, not the spawn-only conclusion this first pass claimed.
  Pitfall: directory capabilities must forbid absolute paths and `..`
  (colluding-sandbox race). Direct syscalls on capabilities cost only a few
  percent vs pure message passing, which supports monolithic plus handles over
  microkernel IPC. [Capsicum paper](https://research.google.com/pubs/archive/36736.pdf)

### Filesystem design (for WizFS)

- **bcachefs** (WizFS's closest ancestor): snapshots as **key versioning**,
  every key is (inode, offset, snapshot_id), so snapshot creation is O(1)
  with no tree cloning (btrfs's scalability pain). Big log-structured btree
  nodes (128 to 256K) with their own node cache give shallow trees.
  **Checksums are stored in the btree with the pointers, not next to data**,
  because co-located checksums cannot catch lost writes or stale reads. Btree updates are journaled; one ordered journal gives
  prefix consistency; about 48 recovery passes run over a dependency graph.
  [principles of operation](https://bcachefs.org/bcachefs-principles-of-operation.pdf)
- **btrfs**: the CoW-friendly B-tree recipe is top-down updates, no leaf
  chaining and lazy reference counting; back-references for every extent
  (defrag and relocation with snapshots, at 2× pointer cost); a separate
  log-tree for fsync latency, because full-checkpoint fsync is too slow in CoW.
  [Rodeh et al.](https://dl.acm.org/doi/10.1145/2501620.2501623)
- **ZFS**: Merkle checksums and self-healing; clones a separate primitive from
  immutable snapshots; checksums without scrub/repair are insufficient; the
  parity RAID write-hole is a trap (skip parity RAID).
- **BetrFS/Bε-trees**: write-optimized dictionaries give 1 to 2 orders of
  magnitude on microwrites, with "copy-on-abundant-write" clones. **POSIX
  preconditions force point-queries before writes; our non-POSIX native ABI
  can expose blind upserts** and keep the write-optimized fast path. Full-path
  keying makes rename/delete expensive (keep inode-style indirection).
  [BetrFS clones (CAW)](https://par.nsf.gov/biblio/10298724-copy-abundant-write-nimble-file-system-clones)

### WASI / WASM (app format)

- **WASI 0.3 is ratified (2026)** with native async that is
  **completion-based and host-driven**, which maps almost 1:1 onto our native
  queue ABI.
  Target 0.3, skip 0.2's poll-based pattern. [Bytecode Alliance](https://bytecodealliance.org/articles/WASI-0.3)
- **wasmi 1.0** (Dec 2025): API-stable, **no_std**, register-machine
  interpreter, differential-fuzzed against wasmtime: the right first runtime.
  It has no WASI/component support, so we implement WASI host bindings
  **directly over native handles** (wasmtime's cap-std/WasiCtx pattern shows
  the shape; its tokio/std coupling makes it unusable in-kernel). [wasmi 1.0](https://wasmi-labs.github.io/blog/posts/wasmi-v1.0/)

### Kernel async executor

Waker-based executor (never busy-poll); no allocation or blocking in interrupt
handlers feeding it; handle the lost-wakeup race with re-check patterns;
futures are self-referential → `Pin`. Cooperative tasks share one stack but can
monopolize the CPU, so pair them with preemptive kernel threads for untrusted
work. [phil-opp: Async/Await](https://os.phil-opp.com/async-await/)

### Scheduler

sched_ext shows pluggable schedulers at EEVDF parity with crash-fallback. Keep
EEVDF-style virtual deadlines in-kernel; a pluggable-policy hook is a cheap
future door.

## Gaps (no confirmed claims — needs a follow-up pass)

CHERI/CET/PKS hardware security, memory tiering/folios, Redox/Managarm/
SerenityOS retrospectives, seL4 verification tooling (Kani/Verus), and
independent replication of the Asterinas numbers. The 14 unverified
claims above should be re-verified when convenient.

## Actions adopted into 002-architecture.md

1. **Framekernel discipline**: kernel splits into a privileged `wost`
   framework crate (all unsafe: paging, MMIO, port I/O, DMA, context
   switch, boot) and `#![forbid(unsafe_code)]` service crates.
2. **Policy layer**: handle-attached rights + Zircon-style job/process
   policy for handle-less syscalls (creation, ring setup).
3. **Fault containment**: supervised restartable services; unwinding is
   best-effort diagnostics, never the containment mechanism.
4. **Ring ABI**: explicit structure versioning; minimal opcode set;
   defensive validation of all shared-memory inputs.
5. **WizFS**: key-versioned O(1) snapshots (bcachefs-style), checksums
   in-btree with pointers, journaled btree updates, blind-upsert native
   API (no POSIX precondition queries), no parity RAID.
6. **WASI**: target WASI 0.3 completion-based async over native queues;
   wasmi-class interpreter first; custom WASI host over handles.
7. **Executor**: waker-based kernel executor rules (no IRQ allocation,
   lost-wakeup re-check, `Pin` discipline).
