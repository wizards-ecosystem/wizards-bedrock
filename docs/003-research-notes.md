# 003 — Deep Research: State-of-the-Art Kernel & OS Architecture (2026)

> **Historical research snapshot.** This is the evidence available at the
> founding design pass, not the current normative architecture. `010` re-checks
> it and corrects the framekernel metric and performance claim, the async-ABI
> rationale, Capsicum's rights count, the WASI target, and two WizFS rules.
> Current decisions live in `004-decision-log.md` and `002-architecture.md`.

Produced 2026-07-18 by a multi-agent research pass (5 search angles, 26
sources fetched, 125 claims extracted, 25 adversarially verified: 11
confirmed, 0 refuted, 14 unverified due to verification-infrastructure
interruptions — flagged below). Findings feed `002-architecture.md`; the
"Actions" section at the bottom lists the concrete design changes adopted.

## Tier 1 — Verified findings (adversarial 3-vote confirmation)

### 1. Zircon's handle-centric authorization is a proven template (3-0)

Primary Fuchsia docs confirm: kernel objects perform **no security checks
themselves**; all rights live on per-process handles ("an active session
with a specific OS subsystem scoped to a particular resource"). One
important qualification: Zircon is not 100% handle-mediated — handle-less
creation calls (e.g. `zx_channel_create`) are governed by **Job policy**
(`zx_job_set_policy`), a per-container escape hatch.
→ *Adopt handle-attached rights AND an explicit process/job policy layer
for handle-less operations.*
Sources: [fuchsia.dev kernel objects](https://fuchsia.dev/fuchsia-src/reference/kernel_objects/objects),
[kernel concepts](https://fuchsia.dev/fuchsia-src/concepts/kernel/concepts)

### 2. The framekernel (Asterinas, USENIX ATC'25) is the validated Rust-kernel architecture (3-0 ×3)

Single address space, monolithic performance (plain function calls, no
IPC), but **all `unsafe` confined to a small privileged framework (OSTD)**;
every service — processes, scheduler, FS, network, drivers — is written in
**100% safe Rust**. Memory-safety TCB ≈ **14.0%** of the codebase vs
Tock 43.8% / Theseus 62.4% / RedLeaf 66.1% (2-1 verified; self-measured by
the authors). Delivers 210+ Linux syscalls at ~Linux-parity single-core
performance (LMbench geomean 1.08, Nginx 1.17, Redis 1.31, SQLite 0.85 —
caveats: single-core only, Linux 5.15, mitigations=off, smoltcp lacks
congestion control).
Authors' verified "lessons learned": **retrofitting safety onto legacy C
(Rust-for-Linux) leaves unfixable soundness holes; a clean-slate
safety-first kernel must provide safe MMIO / port-I/O / DMA abstractions
so drivers can be 100% safe Rust.**
→ *This is the strongest single result for us: adopt the framekernel
discipline — a hard privileged-crate boundary — rather than scattering
`unsafe` per subsystem.*
Sources: [ATC'25 paper](https://www.usenix.org/conference/atc25/presentation/peng-yuke),
[arXiv:2506.03876](https://arxiv.org/abs/2506.03876),
[;login: article](https://www.usenix.org/publications/loginonline/asterinas-rust-based-framekernel-reimagine-linux-2020s),
[OSTD book](https://asterinas.github.io/book/ostd/)

### 3. Theseus: language-based isolation works, panic-unwinding recovery does not (3-0 ×2)

Theseus (OSDI'20) runs everything at Ring 0 in one address space using
Rust's type system for isolation — proving language isolation viable for
*in-kernel components*. But its 800K-fault injection campaign recovered
only **69%** of manifested faults; ~76% of failures traced to Rust/LLVM's
**missing asynchronous unwinding**, still open in 2026.
→ *Treat `catch_unwind`-style recovery as best-effort only; design fault
containment as supervised restartable components (Hubris/Zircon lineage).
Keep hardware isolation for arbitrary user code; language isolation is a
kernel-internal tool.*
Sources: [Theseus OSDI'20 PDF](https://www.usenix.org/system/files/osdi20-boos.pdf),
[rust async-unwind issue](https://github.com/rust-lang/unsafe-code-guidelines/issues/211)

## Tier 2 — Sourced but unverified (verification interrupted; primary sources listed — treat as plausible, re-verify before relying on specifics)

### Async ABI (FlexSC / io_uring / IoRing lineage)

- FlexSC (OSDI'10): raw syscall mode-switch is cheap (~150 cycles); the
  *indirect* cost (cache/TLB pollution — up to 65% IPC degradation)
  dominates. Async ABIs win via **batching + locality + core
  specialization**, not by avoiding the trap. [Soares & Stumm](https://www.usenix.org/legacy/event/osdi10/tech/full_papers/Soares.pdf)
- io_uring security record is sobering: ~60% of Google kernelCTF exploit
  submissions targeted io_uring; Google **disables it** on hardened
  targets. → keep our queue opcode surface minimal, no privileged opcodes,
  validate all shared-memory reads defensively (TOCTOU). [kernelCTF rules](https://google.github.io/security-research/kernelctf/rules.html)
- Windows IoRing shipped **explicit structure versioning**; io_uring did
  not (evolves by flags) → version our ring structures from day one.
  Windows also has the kernel map the rings (vs userspace mmap on Linux).
  [comparison](https://windows-internals.com/ioring-vs-io_uring-a-comparison-of-windows-and-linux-implementations/)
- Zircon's async model is **port-centric**: all event sources (signals,
  interrupts, pager faults, user packets) arrive as typed packets in one
  FIFO port; `zx_object_wait_async` is one-shot; ports service thread
  pools. A useful complement/simplification for our completion path.

### Capability mechanics

- Zircon handle = (object ref, rights, owner process); values are
  process-local 32-bit ints; attenuation via `zx_handle_replace`;
  channels are the *sole* delegation mechanism.
- seL4: 10 object types; resource accounting by **retyping Untyped
  memory** (solves kernel-memory DoS elegantly); revocation is
  long-running and needs "incremental consistency" (abortable
  sub-operations). Claimed ~720-cycle IPC. [whitepaper](https://sel4.systems/About/seL4-whitepaper.pdf)
- Capsicum (~60 rights, per-logical-method): capability mode = **denying
  all global namespaces**, which *forced* fork/exec removal (`pdfork` →
  process descriptors) — independent convergent evidence for our
  spawn-only, handle-based process model. Pitfall: directory capabilities
  must forbid absolute paths and `..` (colluding-sandbox race). Fast-path
  direct syscalls on capabilities cost only a few % vs pure message
  passing — supports monolithic + handles over microkernel IPC.
  [Capsicum paper](https://research.google.com/pubs/archive/36736.pdf)

### Filesystem design (for WizFS)

- **bcachefs**: snapshots as **key versioning** — every key is
  (inode, offset, snapshot_id); snapshot creation is O(1), no tree
  cloning (btrfs's scalability pain). Big log-structured btree nodes
  (128–256K) with its own node cache → shallow trees. **Checksums stored
  in the btree with the pointers, not next to data** (co-located
  checksums can't catch lost writes/stale reads). Btree updates are
  journaled; single ordered journal gives prefix consistency; ~48
  recovery passes with dependency graph. [principles of operation](https://bcachefs.org/bcachefs-principles-of-operation.pdf)
- **btrfs**: the CoW-friendly B-tree recipe = top-down updates, no leaf
  chaining, lazy reference counting; back-references for every extent
  (enables defrag/relocation with snapshots, at 2× pointer cost);
  separate log-tree for fsync latency (full-checkpoint fsync is too
  slow in CoW). [Rodeh et al.](https://dl.acm.org/doi/10.1145/2501620.2501623)
- **ZFS**: Merkle chain-of-trust checksums + self-healing; immutable
  snapshots with clones as a separate primitive; checksumming without
  robust scrub/repair machinery is insufficient; parity RAID write-hole
  is a trap (skip parity RAID).
- **BetrFS/Bε-trees**: write-optimized dictionaries give 1–2 orders of
  magnitude on microwrites; "copy-on-abundant-write" clones; key insight
  for us: **POSIX preconditions force point-queries before writes — our
  non-POSIX native ABI can expose blind upserts** and keep the
  write-optimized fast path. Full-path keying makes rename/delete
  expensive (avoid; keep inode-style indirection).

### WASI / WASM (app format)

- **WASI 0.3 is ratified/stable (2026)** with *native async* — and its
  async model is **completion-based, explicitly in the io_uring/IOCP
  lineage, host-driven** → maps almost 1:1 onto our native queue ABI.
  Target 0.3, skip 0.2's poll-based pattern. [Bytecode Alliance](https://bytecodealliance.org/articles/WASI-0.3)
- **wasmi 1.0** (Dec 2025): API-stable, **no_std**, register-machine
  interpreter, differential-fuzzed against wasmtime — the right first
  runtime for us. It has *no* built-in WASI/component support → we
  implement our own WASI host bindings **directly over native handles**
  (wasmtime's cap-std/WasiCtx pattern shows the shape; its tokio/std
  coupling makes it unusable in-kernel). [wasmi 1.0](https://wasmi-labs.github.io/blog/posts/wasmi-v1.0/)

### Kernel async executor (phil-opp lineage)

Waker-based executor (never busy-poll); no allocation/blocking in
interrupt handlers feeding the executor; handle the lost-wakeup race with
re-check patterns; futures are self-referential → `Pin`. Cooperative
tasks share one stack (cheap) but can monopolize the CPU — pair with
preemptive kernel threads for untrusted work.

### Scheduler

sched_ext demonstrates userspace/pluggable schedulers with EEVDF-parity
performance and crash-fallback. For us: keep EEVDF-style virtual
deadlines in-kernel; a pluggable-policy hook is a cheap future door.

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

## Ranked reading list

1. [Asterinas framekernel — USENIX ATC'25](https://arxiv.org/abs/2506.03876) — our architecture template
2. [OSTD book](https://asterinas.github.io/book/ostd/) — the privileged-framework API to study
3. [Zircon kernel objects](https://fuchsia.dev/fuchsia-src/reference/kernel_objects/objects) + [handles](https://fuchsia.dev/fuchsia-src/concepts/kernel/handles) — handle/rights model
4. [bcachefs: Principles of Operation](https://bcachefs.org/bcachefs-principles-of-operation.pdf) — WizFS's closest ancestor
5. [Theseus — OSDI'20](https://www.usenix.org/system/files/osdi20-boos.pdf) — language isolation limits
6. [FlexSC — OSDI'10](https://www.usenix.org/legacy/event/osdi10/tech/full_papers/Soares.pdf) — why async ABIs win
7. [seL4 whitepaper](https://sel4.systems/About/seL4-whitepaper.pdf) — untyped retype + revocation
8. [Capsicum](https://research.google.com/pubs/archive/36736.pdf) — rights taxonomy + no-fork convergence
9. [WASI 0.3 announcement](https://bytecodealliance.org/articles/WASI-0.3) — completion-based component async
10. [Rodeh: BTRFS CoW B-trees](https://dl.acm.org/doi/10.1145/2501620.2501623) — CoW tree mechanics
11. [BetrFS clones (CAW)](https://par.nsf.gov/biblio/10298724-copy-abundant-write-nimble-file-system-clones) — write-optimized clone design
12. [IoRing vs io_uring](https://windows-internals.com/ioring-vs-io_uring-a-comparison-of-windows-and-linux-implementations/) — ring ABI details compared
13. [kernelCTF rules](https://google.github.io/security-research/kernelctf/rules.html) — io_uring's security lesson
14. [wasmi 1.0](https://wasmi-labs.github.io/blog/posts/wasmi-v1.0/) — our likely first WASM interpreter
15. [phil-opp: Async/Await](https://os.phil-opp.com/async-await/) — kernel executor construction
