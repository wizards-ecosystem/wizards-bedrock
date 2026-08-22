# 004 — Decision Log

Architecture Decision Records for The Wizard's OS. Each entry captures the
context, the options weighed, the decision, and its consequences. Statuses:
**Accepted** (settled), **Verified** (settled + supported by the
adversarially-verified research in `003-research-notes.md`).

All decisions below were made 2026-07-18 during the founding design
sessions unless noted.

---

## D-01 Kernel language: Rust — Status: Verified

**Context.** Isaac's fluent language is C++; the project is a multi-year
from-scratch OS. The question was posed as "what is absolute best practice
for modern architecture?"

**Options.** (a) C++ (fluency, SerenityOS/Managarm precedent); (b) Rust;
(c) C++ kernel with Rust later; (d) Rust kernel with C++ userspace.

**Decision.** Rust, on the stable channel for as long as possible.

**Rationale.** The only actively-maintained first-class osdev tutorial
lineage (os.phil-opp.com) and the rust-osdev crate ecosystem are Rust; the
borrow checker eliminates the bug class (UAF, data races) that is hardest
to debug in ring 0; Linux and Windows both accept Rust for new kernel
code. Post-hoc, the research pass strengthened this: Asterinas (USENIX
ATC'25, verified) demonstrates a Rust kernel with a ~14% memory-safety TCB
at Linux-parity single-core performance, and its authors' verified lesson
is that safety must be designed in clean-slate — exactly this project's
position. C++ remains welcome in userspace later.

**Consequences.** Isaac learns Rust alongside OS concepts (slower start,
compounding payoff). Stable-channel discipline occasionally constrains
crate choice (see D-17).

---

## D-02 Target architecture: x86_64 first, ports kept possible — Status: Accepted

**Context.** Host machine is an i7-13700KF (x86_64, UEFI) and testing
happens in VMware/QEMU on it.

**Options.** x86_64; ARM64/Raspberry Pi; RISC-V; multi-arch from day one.

**Decision.** x86_64 exclusively for now, with all arch-specific code
isolated under `kernel .../arch/x86_64/` so ARM64/RISC-V ports remain
feasible without a rewrite.

**Rationale.** Best documentation and tooling; boots on the actual
machine; QEMU + VMware both first-class. RISC-V/ARM are cleaner ISAs but
thinner step-by-step resources; the arch-isolation layer preserves the
option.

**Consequences.** We carry x86 legacy (GDT, APIC quirks); the `arch/`
boundary is a standing discipline every phase must respect.

---

## D-03 Boot: UEFI-only via vendored Limine; own bootloader deferred — Status: Accepted

**Context.** Choice between writing a bootloader (BIOS or UEFI) and
reusing a lean open-source one. "Standing on the shoulders of clean open
source projects" is an explicit project value.

**Options.** (a) Limine now, own later; (b) Limine forever; (c) own UEFI
loader first; (d) classic BIOS boot-sector journey.

**Decision.** Limine (v10 binary, vendored in `boot/limine/` with license
and provenance note), UEFI-only. Writing our own bootloader is a
deliberately deferred side-quest (roadmap Phase 12).

**Rationale.** Limine delivers exactly what a kernel wants: entry in
64-bit long mode, higher-half load, memory map, framebuffer, modules —
identical across QEMU/EDK2, VMware EFI, and real UEFI hardware. BIOS real
mode is historical archaeology on a 2026 machine. A half-working homemade
loader would poison early kernel debugging ("is the bug in the loader or
the kernel?"). Deferring preserves the learning goal at the point where
we'll know precisely what a loader must provide.

**Consequences.** UEFI-only disk layout (no BIOS fallback); dependency on
Limine's boot protocol (base revision 3 — see D-17); `limine.conf` +
`BOOTX64.EFI` are repo artifacts.

---

## D-04 Kernel shape: monolithic-modular — Status: Verified (as refined by D-13)

**Context.** Solo developer, multi-year horizon, full-OS destination.

**Options.** Monolithic-modular; microkernel; hybrid.

**Decision.** Monolithic-modular: drivers, filesystems, and the network
stack live in the kernel as clean internal modules; no userspace-server
IPC tax on the core path.

**Rationale.** Fastest route to a working full OS for one person; the
capability discipline (D-06) applies at the ABI boundary rather than via
process isolation of subsystems. The research pass upgraded rather than
overturned this: the framekernel result (D-13) shows how to get
microkernel-grade safety TCB inside a monolithic design, and Capsicum's
measurements (unverified tier) found direct capability-checked syscalls
cost only a few percent — supporting "fast path over message passing."

**Consequences.** In-kernel bugs can take down the system (mitigated by
D-13/D-14); no seL4-style formal isolation story.

---

## D-05 Destination: full OS with userspace, browser, and Python — Status: Accepted

**Context.** Sets the architecture bar and multi-year scope.

**Decision.** The endpoint is a complete OS: own userland and shell,
networking, GUI with own compositor, and two flagship port ladders —
Python (RustPython early → CPython later) and web browsing (ported
`links` text browser → NetSurf graphical). Pace: massive multi-year
project; milestones favor always-finishable increments.

**Rationale.** CPython and NetSurf are the classic proof-of-real-OS
trophies and force every subsystem (FS, network, graphics, libc) to
become real. RustPython runs pre-libc (pure Rust), giving scripting
powers early; links proves the network stack before graphics exist.

**Consequences.** Demands the POSIX translation layer (D-10) no matter
how novel the native ABI is.

---

## D-06 Native ABI: capability handles + async submission/completion queues — Status: Verified (handle model)

**Context.** Isaac directed the project to be "forward thinking, state of
the art, lightweight end to end, ahead of our time, matching theoretical
frameworks and 2026 standards." The original plan was POSIX-ish syscalls.

**Options.** (a) POSIX-native syscalls; (b) async + capability core with
POSIX as a translation layer; (c) sync now, async ABI v2 later.

**Decision.** (b), reversing the earlier POSIX-ish choice. Every kernel
object is reached through a process-local handle carrying rights bits;
the data-path ABI is shared-memory submission/completion rings; a small
synchronous syscall set covers setup, handle operations, mapping, and
waits. A per-process **policy layer** governs handle-less calls (object
creation, ring setup).

**Rationale.** io_uring is the 2026 performance standard and FlexSC
(OSDI'10) explains why (batching + cache locality, not trap avoidance —
unverified tier). Zircon's handle-centric authorization is verified as
Fuchsia's deployed design; the policy escape hatch mirrors Zircon Job
policy (verified caveat). Capsicum independently converged on
capabilities + no global namespaces (unverified tier).

**Ring-ABI rules adopted from io_uring's security record** (Google
kernelCTF disables io_uring; ~60% of exploit submissions targeted it —
unverified tier): explicit version fields in ring structures (IoRing's
improvement), minimal opcode surface, no privileged opcodes, defensive
validation of everything read from shared memory.

**Consequences.** More design work before first userspace (handle table,
rights taxonomy, queue layout — each gets a doc). POSIX becomes a
userspace translation library rather than the kernel's shape. WASI
integration (D-09) becomes nearly free.

---

## D-07 Process model: spawn-only, no native fork — Status: Accepted

**Context.** Follows from D-06; `fork()` duplicates ambient authority and
is unsound with threads.

**Options.** Classic fork/exec; spawn-only with compat-fork emulation.

**Decision.** Native process creation is
`spawn(image, initial_handle_set, args)`; the parent explicitly grants
the child its starting capabilities. No native fork. The kernel provides
an address-space-clone primitive gated as a compat capability, used only
by the POSIX layer for ports that genuinely need fork.

**Rationale.** "A fork() in the road" (HotOS'19) crystallized the
consensus; Fuchsia shipped it; Capsicum's capability mode *forced* the
same conclusion independently (pdfork). Fork is incompatible with
"a process's power is exactly its handle table."

**Consequences.** mlibc's fork-dependent ports get the slower emulated
path; native software uses spawn and is better for it.

---

## D-08 Filesystem: custom WizFS, CoW from the start — Status: Accepted (design rules from research)

**Context.** Isaac chose a custom filesystem over adopting ext2, at the
"straight to custom FS" level, then raised the bar to 2026 standards.

**Options.** ext2 clone first; simple extent FS v1 then CoW v2; CoW
checksummed design from the start.

**Decision.** WizFS is copy-on-write, checksummed, snapshotting, designed
once (no throwaway v1), implemented behind a VFS with initrd/tmpfs/devfs
as stepping stones. Design rules locked from the research pass
(bcachefs/btrfs/ZFS/BetrFS evidence, unverified tier but primary-source
backed):

- snapshots as **key versioning** — keys are (inode, offset,
  snapshot_id); O(1) snapshot creation, no tree cloning (bcachefs
  design; avoids btrfs's scalability pain)
- **checksums stored in the btree beside pointers**, never co-located
  with data — co-located checksums cannot detect lost writes
- **journaled btree updates**, single totally-ordered journal → prefix
  consistency after crash; large log-structured nodes with our own cache
- **blind upserts in the native API** — the handle ABI (D-06) has no
  POSIX preconditions, so the write-optimized path needs no
  read-before-write (BetrFS lesson)
- inode-style indirection (not full-path keying — rename/delete cost)
- scrub/repair machinery designed in from the start; no parity RAID ever
  (write hole)

**Risk management.** The on-disk format lives in `libs/wizfs-format`, a
no_std crate that also compiles on the host — unit-tested and fuzzed on
Windows against the same code the kernel mounts. WizFS must carry POSIX
semantics (permissions, symlinks, timestamps) or the D-10 ports suffer.

**Consequences.** Highest-risk subsystem in the project (custom FS eats
data when wrong); mitigations above are non-negotiable discipline.

---

## D-09 App formats: ELF64 + WASI 0.3 components as first-class — Status: Accepted

**Context.** "Ahead of its time" directive; sandboxing story needed.

**Options.** ELF only; design-for-WASM-later; WASI first-class now.

**Decision.** Two first-class formats: native ELF64, and WASI Component
Model apps. Target **WASI 0.3** (ratified 2026): its async model is
completion-based and host-driven — the io_uring/IOCP lineage — mapping
nearly 1:1 onto our native queues. First runtime: **wasmi 1.0**
(API-stable Dec 2025, no_std, register-machine interpreter,
differentially fuzzed against wasmtime). WASI host bindings are written
by us directly over native handles; wasmtime's host implementation is
tokio/std-coupled and unusable in-kernel, but its cap-std/WasiCtx
pattern is the architectural shape to follow.

**Rationale.** A WASI descriptor is literally a capability grant — the
component model *is* our handle model in portable form. Sandboxed,
portable apps without porting a whole toolchain.

**Consequences.** The syscall/handle layer must be designed so the WASI
host is a pure userspace/kernel-service consumer of it (no special
kernel hooks); wasmi has no built-in WASI so the host is our code.

---

## D-10 POSIX as a translation layer only; mlibc is the ports gateway — Status: Accepted

**Context.** CPython and NetSurf (D-05) require POSIX; the native ABI
(D-06) is not POSIX.

**Decision.** POSIX lives entirely in userspace: mlibc's sysdeps
implemented over the native ABI (fds map to handles; blocking calls wrap
queue submissions + waits; fork via the gated compat clone). Native
software never links it.

**Rationale.** mlibc exists precisely for hobby-OS ports (Managarm
lineage); this preserves both the modern core and the porting trophies.

**Consequences.** Phase 9 is the steepest climb in the roadmap; the
Phase-5 ABI design doc must be written with mlibc's sysdeps checklist
open so no primitive is missing.

---

## D-11 Graphics: own compositor, Wayland-style protocol, GPU-first toolkit — Status: Accepted

**Context.** Isaac wants his own windowing system, "probably Wayland
style, but maybe something way more custom."

**Decision.** Own compositor and window protocol in the Wayland model —
clients render into shared `MemoryObject` buffers, the compositor
composites; protocol details free to diverge from Wayland where we have
better ideas. Native widget toolkit renders GPU-first (wgpu/vello-style
compute 2D) targeting virtio-gpu in VMs, dumb-framebuffer fallback.
NetSurf gets a toolkit/framebuffer backend when its phase arrives.

**Rationale.** Wayland's buffer-sharing model is the proven 2026 shape
and maps directly onto our memory-object handles; GPU-compute 2D is
where rendering is headed.

**Consequences.** GUI phase depends on memory objects + channels being
solid; a terminal emulator is the first GUI client.

---

## D-12 Security baseline from day one — Status: Accepted

**Decision.** W^X for every mapping, NX, SMEP + SMAP + UMIP in CR4
during early boot, KASLR (via Limine) once addresses aren't hardcoded,
guard pages under all stacks, zero-on-allocate. Later: Intel CET shadow
stacks, signed/measured boot.

**Rationale.** 2026 table stakes; retrofitting mitigations is how
systems end up with exceptions. Costs almost nothing at this codebase
size; costs everything later.

**Consequences.** Phase 2/3 code must set this up before conveniences
(e.g. no RWX "temporary" mappings, ever).

---

## D-13 Framekernel discipline: all `unsafe` in one privileged crate — Status: Verified

**Context.** Research pass, strongest verified result (Asterinas,
USENIX ATC'25).

**Decision.** The kernel is one address space with monolithic
performance, but ALL `unsafe` Rust lives in a small privileged framework
crate — **`wost`** (Wizard OS Standard, our OSTD analogue): boot,
paging, MMIO, port I/O, DMA, context switch, interrupt entry. Every
other kernel crate (scheduler, VFS, WizFS, network, drivers) is
`#![forbid(unsafe_code)]` safe Rust written against `wost`'s safe
abstractions.

**Rationale.** Verified: this yields a microkernel-sized memory-safety
TCB (~14% in Asterinas vs 43–66% in Tock/Theseus/RedLeaf) at
Linux-parity single-core performance; the authors' verified lesson is
that safe MMIO/port/DMA abstractions must exist from day one.

**Consequences.** The existing Phase-0 kernel gets restructured into
`wost` + service crates before Phase 2 lands significant code. Safe DMA
is genuinely hard (ownership of in-flight buffers) — `wost`'s API
design is a first-class engineering problem, not plumbing.

---

## D-14 Fault containment by supervision, not unwinding — Status: Verified

**Context.** Theseus OSDI'20 fault-injection data (verified): 800K-fault
campaign, only 69% recovery, failures dominated by Rust/LLVM's missing
asynchronous unwinding (still open in 2026).

**Decision.** Services are designed restartable under a supervisor;
`catch_unwind` is diagnostics only, never the containment mechanism.
Hardware isolation (rings + address spaces) remains the boundary for all
user code; language safety is a kernel-internal tool.

**Consequences.** Subsystems keep restartable state boundaries
(Hubris/Zircon lineage); panic = log + supervised restart or halt,
never "unwind and hope."

---

## D-15 Scheduler: EEVDF-inspired virtual deadlines, tickless — Status: Accepted

**Decision.** Virtual-deadline fair scheduling (the EEVDF family Linux
adopted in 6.6), tickless idle (no periodic timer when nothing runs).
A pluggable-policy hook (sched_ext's lesson) is a cheap future door,
not a v1 feature.

**Rationale.** Modern fairness with modest implementation complexity;
zero idle wakeups serves the "lightweight" mandate directly.

---

## D-16 Fully self-contained native-Windows build — Status: Accepted

**Context.** Initial toolchain work assumed WSL2; Isaac rejected it:
"I prefer to keep everything self contained."

**Options.** WSL2 toolchain; native Windows + system installs; native
Windows + everything project-local.

**Decision.** Native Windows, minimal installs, everything else
in-repo: rustup with the **GNU host toolchain** (no Visual Studio on the
machine; MSVC not required), QEMU via winget (approved system install),
and **pure-Rust disk imaging inside `xtask`** — hand-rolled MBR
partition table + FAT32 ESP via the `fatfs` crate. No WSL, no xorriso,
no mtools, no external imaging tools. VMware artifacts (flat-VMDK
descriptor + EFI `.vmx`) are also generated by `xtask` as plain text.

**Rationale.** The kernel is a cross-compiled freestanding ELF — Rust
does that natively from Windows with its bundled `rust-lld`. Imaging is
just bytes; owning it in Rust removes an entire class of environment
fragility and matches the design-first ethos (we know exactly what's on
disk, byte by byte).

**Consequences.** MBR (not GPT) partition table for now — deliberately
minimal, hand-written, UEFI boots it fine; GPT arrives when the OS
grows its own partitioning story. Ask before any new system-level
install.

---

## D-17 Toolchain pins and workarounds — Status: Accepted (revisit periodically)

- **Rust 1.97.1**, pinned exactly via `rust-toolchain.toml`
  (`profile = "minimal"`, `rustfmt`/`clippy`/`rust-mingw`, the
  `x86_64-unknown-none` target).
  *Corrected 2026-08-12: this bullet previously read "stable (1.97.x), pinned"
  while the file actually said `channel = "stable"` — a floating channel. See
  D-24.*
- **`limine` crate = 0.5.0 exactly**: 0.6.x requires nightly
  (`ptr_metadata`). 0.5's base revision 3 is confirmed working against
  the vendored Limine v10 binary (verified by boot on QEMU + VMware).
  Revisit when 0.6's requirement lands on stable or if we adopt nightly.
- **`fatfs` without its `chrono` feature**: chrono's Windows bindings
  need `dlltool.exe`, absent from the self-bundled GNU toolchain. Disk
  images get fixed timestamps — irrelevant for boot media.
- **Kernel link flags**: `code-model=kernel`, `relocation-model=static`
  (higher-half at `0xffffffff80000000`); linker script passed via
  `build.rs` absolute path so builds work from any directory.

---

## D-18 Test targets: QEMU is the loop, VMware is the milestone — Status: Accepted

**Context.** Isaac wanted VMware as the primary test target; QEMU's
debugging story argued for a role.

**Decision.** QEMU (q35 + bundled EDK2 UEFI firmware, serial → stdio)
is the minute-to-minute dev loop: `cargo xtask run`. VMware Workstation
(EFI firmware, serial → `vm/serial.log`, driven manually or via
`vmrun`) is the end-of-phase "does it feel real" gate: `cargo xtask
vmware`. Every phase's definition-of-done includes a VMware boot.

**Consequences.** Two firmware/NIC/GPU stacks exercised continuously
(e1000 + SVGA-II on VMware, virtio on QEMU) — drivers get two targets
from day one. GDB-stub debugging is an open item (no native Windows
gdb yet).

---

## D-19 Method and voice — Status: Accepted

**Decision.** *Design-first*: every subsystem gets a numbered doc in
`docs/` before implementation, citing references (OSDev wiki,
Osdev-Notes, Intel SDM, phil-opp as reference — never followed
linearly). *Theme*: wizard/magic identity lives at the product level
(app names, shell personality, boot messages — e.g. the serial banner
"The Wizard's OS awakens"); code uses conventional professional naming
throughout. Docs are numbered in creation order and never renumbered.

**Rationale.** Multi-year projects survive on re-enterable context;
docs-before-code makes every subsystem resumable after months away.
Theme-in-code ages badly; theme-in-product is charm.

---

## D-20 Networking plan: smoltcp + rustls in-kernel — Status: Accepted (planned)

**Decision.** The TCP/IP stack is [smoltcp](https://github.com/smoltcp-rs/smoltcp)
(production bare-metal Rust, also Asterinas's choice) integrated
in-kernel behind async `Socket` handles; TLS via rustls in userspace.
Drivers: virtio-net (QEMU) and e1000 (VMware) first.

**Known caveat** (research pass): smoltcp lacks TCP congestion control —
fine for LAN/VM development, must be revisited before real-network
workloads are a claim we make.

---

## D-21 The framekernel crate split lands now, not later — Status: Accepted (2026-08-12)

**Context.** D-13 committed to Asterinas-style framekernel discipline: all
`unsafe` confined to a privileged framework crate, every other kernel crate
safe. The Asterinas authors' explicit lesson is that safe MMIO/DMA
abstractions must be designed in from day one, not retrofitted. At the end
of Phase 0 the kernel was 242 lines with `unsafe` spread across two peer
driver modules, no framework crate, and no `// SAFETY:` comment anywhere.

**Options.** (a) Defer the split until there is more kernel to split.
(b) Impose the layering as module structure inside one crate, so the boundary
is convention. (c) Create `wost/` now, while the whole kernel is 242 lines.

**Decision.** (c). `wost/` owns boot handoff, port I/O, MMIO, CPU control,
locking and the panic token. `kernel/` carries `#![deny(unsafe_code)]` and has
no hand-written `unsafe` at all: the UART driver, console, logging, and panic
reporting are ordinary safe Rust over `wost`'s abstractions. `BootInfo` and
`Framebuffer` remove `limine` from the kernel's dependency list entirely.

**Rationale.** This is the cheapest the migration will ever be, and doing it
later means retrofitting exactly the abstractions the research says cannot be
retrofitted. The split immediately paid for itself: `MmioRegion`'s bounds
check is what makes a non-32-bpp framebuffer a typed error instead of an
out-of-bounds write, and `port::claim`'s registry turns "no other driver owns
these ports" from a comment into a checked fact.

**Consequences.**

- **`kernel/` uses `deny(unsafe_code)`, not `forbid`, amending D-13.**
  `wost::entry!` expands `#[allow(unsafe_code)]` glue into the binary crate,
  and `forbid` cannot be lifted by an `allow` anywhere, including in a macro
  expansion — rustc rejects it outright. Future service crates (scheduler,
  VFS, drivers) have no entry glue and keep `forbid`.
- **The entry glue expands in the binary crate, not in `wost`.** `#[used]`
  statics inside an rlib can be dropped wholesale by the linker: nothing
  references the archive member holding them, so `#[used]` never gets a chance
  to protect it (rust-lang/rust#47384). Every `-rt` crate in the ecosystem
  expands entry glue in the binary for this reason. `linker.ld` and `build.rs`
  stay in `kernel/` too — a build script's `rustc-link-arg` does not propagate
  to dependents, so a `wost/build.rs` emitting `-T linker.ld` would silently do
  nothing.
- `ENTRY(kmain)` is unchanged; the macro still emits a symbol named `kmain`.
- Verified 2026-08-12 on QEMU: identical serial banner, framebuffer background
  byte-identical at `0x0B1021`, panic screen and double-panic guard exercised,
  scrolling verified over 60 lines.

---

## D-22 Lint, test, and CI policy — Status: Accepted (2026-08-12)

**Decision.** A `[workspace.lints]` table all kernel crates opt into:
`unsafe_op_in_unsafe_fn`, `unused_must_use` denied; `missing_docs`,
`unreachable_pub` at warn; and from clippy's allow-by-default `restriction`
group, `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`, and
`missing_safety_doc` denied. `clippy::missing_docs_in_private_items` is
deliberately **not** enabled. `cargo xtask ci` is the single gate.
*(A GitHub Actions workflow ran the same steps on Linux for a day; removed by
D-26.)*

**Rationale.** `undocumented_unsafe_blocks` is what mechanically forces a
`// SAFETY:` on every unsafe operation, and its companion keeps each comment
covering exactly one operation. Together they turn D-13's discipline from an
aspiration into a build failure. The excluded lint is the one that would
demand a doc comment on every private helper, which is noise; the policy is
"prose where it carries a safety obligation, nowhere else"
(`005-code-conventions.md`).

**Amended 2026-08-13.** The gate was linting only the library targets: neither
`check` nor `clippy` passed `--all-targets`, so no test code in the workspace
had ever been linted, and a `useless_vec` in `sha256.rs`'s tests had been sitting
there unreported. `--all-targets` now goes to the host half — and only the host
half, because the kernel crates have no test or bench targets and cannot have
any (D-28). The argument construction moved into a pure `half_args` function
with tests, so the distinction is asserted rather than remembered.

**Amended 2026-08-22, after an adversarial audit.** Three holes, all of the
same species as the one above — the gate covering less than it was believed
to.

1. *The configuration that ships was never linted.* Every kernel pass carried
   `--features wizard-kernel/selftest`, so `cargo xtask ci` linted exactly one
   crate graph and the release build in its own last step linted none. Two
   `dead_code` constants used only by `selftest` had been riding release
   images out, printed as warnings by a step nothing gated. The pass list is
   now an explicit `PASSES` table — host, kernel, kernel + self-test — with
   tests asserting the kernel appears twice and that the shipped
   configuration comes first.
2. *`kernel/` never opted in at all.* It had no `[lints]` table, so every lint
   this ADR describes was unenforced in the crate that most needed them — the
   `unreachable_pub` that `005` says enforces `pub(crate)` in a binary was
   enforcing nothing. It opts in now. The code turned out to already conform,
   which is the good outcome and not the point: it was conforming by habit.
3. *Doc links rot unwatched.* `missing_docs` catches an absent doc; nothing
   caught one pointing at a renamed or now-private item, because those lints
   fire only under rustdoc and no gate step ran it. `[workspace.lints.rustdoc]`
   denies `broken_intra_doc_links`, `private_intra_doc_links` and
   `redundant_explicit_links`, and `cargo xtask doc` makes them bite. Two dead
   links were found on the first run. `xtask` takes this table on its own,
   still opting out of the rest.
4. *The package list drifted, because there are two of them.* `HOST_PACKAGES`
   in `xtask/src/paths.rs` and `default-members` in the root manifest say the
   same thing twice, and adding `libs/port-space` to one and not the other
   made the gate stop covering it while a bare `cargo test` still did — found
   while fixing hole 1, by the same kind of looking. `paths::default_members`
   now reads the real manifest in a unit test and requires the two to agree.
   A list duplicated without a check between the copies is a defect with a
   waiting period.

**Consequences.** Cargo forbids mixing `[lints] workspace = true` with local
lint tables in the same manifest, so per-crate escalation goes in source
attributes — `#![deny(unsafe_code)]` sits at the top of `kernel/src/main.rs`,
where a reader meets it in the first five lines. `xtask` opts out of the
workspace lints bar the rustdoc ones: `missing_docs` on build tooling is
noise, a link that no longer resolves is a defect anywhere. Automated tests
exist for host-side logic only; the reasoning and the path forward are in
`005-code-conventions.md`.

The standing lesson, now twice learned: **a gate is only as wide as its
configuration matrix, and the matrix is the thing to audit.** Every widening
so far has been found by looking, never by the gate itself.

---

## D-23 xtask is a library with a thin binary — Status: Accepted (2026-08-12)

**Context.** `xtask/src/main.rs` had grown to 254 lines in one file, with
hardcoded paths, twelve fallible calls carrying no context, and no tests. The
one arithmetic defect it contained — a truncating cylinder division that
declared a VMDK geometry 1.25 MiB smaller than its extent — was invisible
precisely because nothing could exercise it.

**Decision.** Split into `lib.rs` plus modules (`cli`, `paths`, `cargo`,
`tasks`, `image/{mbr,esp}`, `qemu`, `vmware`, `vendor`, `sha256`) with a
~15-line `main.rs`. Pure byte-layout and arithmetic live in functions that
take values and return values, tested in-module. Package names, target triple,
and profile are constants in `paths`, and every path is derived from them.
Argument parsing is hand-rolled — twelve tasks and six flags are a testable
`match`, not a reason to take `clap` and three transitive crates.

**Consequences.** 42 host tests, including a byte-exact MBR assertion and a
regression test for the cylinder rounding. `CARGO_TARGET_DIR` is honoured and
`--debug` builds now find their artifact. `std::env::consts::EXE_SUFFIX` and
`QEMU`/`QEMU_DIR` overrides replace the hardcoded `.exe` and install path, so
the tooling is not Windows-only. New tasks: `check`, `clippy`, `fmt`, `test`,
`verify-vendor`, `ci`, `clean`. **A bare `cargo xtask` is now an error printing
usage** — the old default silently booted a virtual machine whenever a task
name was mistyped.

---

## D-24 The toolchain pin is exact — Status: Accepted (2026-08-12)

**Context.** D-17 recorded the toolchain as "pinned (1.97.x)" while
`rust-toolchain.toml` actually said `channel = "stable"`.

**Decision.** `channel = "1.97.1"`, `profile = "minimal"`. D-17's bullet is
corrected in place.

**Rationale.** Clippy gains lints every six weeks. With the lint policy of
D-22 gating the build, a floating channel means CI can go red because a
toolchain shipped, not because a commit did — and a reproducible OS build is
worth more than automatic updates. Bump deliberately.

**Consequences.** `components` lists only `rustfmt` and `clippy`. `rust-mingw`
was listed too at first, on the theory that the minimal profile might drop a
component the windows-gnu host needs to link — but naming it breaks CI outright,
because it does not exist for a Linux host and rustup resolves `components`
against whatever host it is running on. It is unnecessary anyway: rustup
installs it as part of the windows-gnu host toolchain regardless of profile
(verified with `rustup component list --installed` after the pin landed).
Anything host-specific belongs in the workflow or the README, never in
`rust-toolchain.toml`, which every host reads.

---

## D-25 Vendored binaries are checksum-pinned — Status: Accepted (2026-08-12)

**Context.** `boot/limine/BOOTX64.EFI` is a 300 KB prebuilt binary from
another project, recorded with provenance and a fetch date but no digest, and
referencing a rolling `v10.x-binary` branch. An update was unverifiable.

**Decision.** `boot/limine/CHECKSUMS` records a SHA-256 in `sha256sum` format,
checked by `cargo xtask verify-vendor`, by `cargo xtask ci`, and by an
integration test so that tampering fails `cargo test` too. The hash function
is ~90 lines in `xtask/src/sha256.rs`, verified against the NIST vectors —
cheaper than a dependency for one call site.

**Amended 2026-08-22.** A digest catches a vendored file being *changed*; it
says nothing about one being *added*, and an unlisted binary appearing beside
`BOOTX64.EFI` passed the check silently. `verify_listing` now reads the
directory as well as the listing and refuses anything neither an entry nor the
directory's own paperwork — `CHECKSUMS`, `LICENSE`, `README.md` — accounts
for. The comparison is a pure `unaccounted` function, tested on the host;
the check was broken once with a stray `EXTRA.EFI` and went red.

**Consequences.** Updating Limine is now a two-line diff a reviewer can
actually check. Every future vendored artifact gets the same treatment.

---

## D-26 No hosted CI; the local gate is the gate — Status: Accepted (2026-08-12)

**Context.** A GitHub Actions workflow ran `fmt --check`, clippy, tests, the
vendor digests, and an image build on Linux for every push.

**Decision.** Removed. `cargo xtask ci` is the only gate.

**Rationale.** This is a single-developer project on a single machine, and the
workflow ran exactly the steps `cargo xtask ci` already runs locally in about
twenty seconds — before the push rather than after it. On a private repository
it also consumes a metered minute allowance to tell one person something they
already knew. Branch protection is rejected for the same reason: with no second
author there is no review to enforce, only a barrier to `git push`.

**Consequences.** Nothing now proves the build works on a non-Windows host. That
guarantee was real — it is why `xtask` uses `EXE_SUFFIX` and `QEMU_DIR` instead
of hardcoding `.exe` and a Program Files path — so the portability *code* stays,
untested. Revisit the moment a second person contributes, or if this is ever
built on Linux in earnest; the deleted workflow is recoverable from history.

---

## D-27 The Rust API Guidelines are the house style — Status: Accepted (2026-08-12)

**Context.** `005-code-conventions.md` was written from the project's own
experience. Before building out subsystems on top of it, it was worth checking
against the established sources rather than inventing a local dialect.

**Decision.** Adopt the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
as the default standard, and the [Rust for Linux coding
guidelines](https://docs.kernel.org/rust/coding-guidelines.html) where they are
more specific about kernel work — chiefly the `// SAFETY:` / `# Safety`
distinction and the rule against repeating a namespace inside an identifier.
`005` now cites the specific checklist items it turns into rules.

**Rationale.** These are the conventions a Rust programmer already has in their
head. Matching them means the code reads correctly to someone who has never
seen this project, and disagreements have an external referee rather than being
a matter of taste.

**Consequences.** Four guideline violations were found and fixed in the existing
code: `missing_debug_implementations` added to the lint table and `Debug`
implemented across `wost` (C-DEBUG); `Display` and `core::error::Error` on every
error type (C-GOOD-ERR); `bool` parameters replaced with named types —
`FmtMode`, `CleanScope`, and a `Palette` that stops two same-typed `Rgb`
arguments being passed the wrong way round (C-CUSTOM-TYPE); and `#[non_exhaustive]`
on the error enums (C-STRUCT-PRIVATE's future-proofing rationale).

Two guidelines are explicitly **not** adopted, with reasons recorded in `005`:
C-EXAMPLE (a rustdoc example on every item) because `no_std` cross-compiled
crates cannot run doctests, and Rust for Linux's one-import-per-line layout
because it contradicts the `rustfmt` defaults this project formats with.

---

## D-28 On-machine tests via QEMU's exit device — Status: Accepted (2026-08-12)

**Context.** The kernel had no automated check of any kind, and by construction
could not have one: `cargo test` cannot host a `#![no_std] #![no_main]` binary,
and the in-QEMU `#[test_case]` harnesses need nightly's
`custom_test_frameworks`, which D-01 rules out. Phase 1 got away with this
because a banner can be verified by looking at it. Phase 2 cannot — nobody
eyeballs an interrupt handler.

**Options.** (a) Keep extracting logic to `libs/` and accept that anything
touching hardware is untested. (b) Adopt nightly for `custom_test_frameworks`.
(c) A `selftest` cargo feature plus QEMU's `isa-debug-exit` device.

**Decision.** (c). A kernel built with `--features selftest` runs
`kernel/src/selftest.rs` after boot and writes its verdict to port `0xF4`;
QEMU exits with `(value << 1) | 1`, so 0x10 becomes 33 and 0x11 becomes 35.
`cargo xtask test-boot` builds that kernel, boots it headless under a timeout,
and maps the exit code to a pass or a failure. It is part of `cargo xtask ci`.

**Rationale.** Writing to a port is ordinary port I/O through an already-safe
`Port`, so this needs no nightly feature, no custom test runner, and no `unsafe`
in the kernel. It is the only mechanism that can test interrupt entry, paging,
and context switching — all of which arrive in the next two phases.

**Consequences.** The feature is never enabled in a shipped image, and
`debug_exit::report` is a no-op when the device is absent, so a self-test kernel
booted on real hardware still reaches its idle loop rather than hanging. The
harness treats a hang as a failure via its own timeout, which is what a panic
during boot looks like from outside. Six checks exist today — serial and console
bring-up, framebuffer geometry, non-overlapping colour masks, port-claim
exclusivity, and lock exclusion; the harness was verified by breaking one and
confirming it goes red.

**Not covered.** Anything whose failure is visual rather than logical. The
VMware boot of D-18 remains the definition of done for a phase.

---

## D-29 Interrupt entry is generated assembly, not a nightly ABI — Status: Accepted (2026-08-13)

**Context.** Phase 2 needs 256 interrupt gates. Rust cannot express a handler
directly: the calling convention that would let it, `extern "x86-interrupt"`,
is gated behind nightly's `abi_x86_interrupt`, and D-01 keeps this project on
stable.

**Options.** (a) Adopt nightly for that one feature. (b) Take the `x86_64`
crate, which wraps the ABI and ships a full descriptor-table API. (c) Write the
entry path in assembly ourselves.

**Decision.** (c). One `global_asm!` block generates 256 stubs with `.rept`,
each padded to sixteen bytes so the table is indexed by vector rather than
naming 256 functions; each stub normalises the frame — pushing a zero where the
CPU pushes no error code, then its own vector number — and jumps to a common
tail that saves the fifteen general-purpose registers and calls a Rust
dispatcher with a pointer to the frame.

**Rationale.** Nightly for one feature costs the whole stable-channel
guarantee. The `x86_64` crate would be the pragmatic choice in most projects,
but it inverts D-13: the `unsafe` that defines this kernel's TCB would live in
a dependency rather than in `wost`, and the framekernel argument is precisely
that the privileged surface is small, ours, and reviewable. Writing the entry
path is also the part of Phase 2 most worth understanding by hand.

**Consequences.**

- **The stub stride is an assumption, so it is checked.** The generated table is
  bracketed by two symbols and `init` asserts the span is exactly `256 × 16`
  bytes before indexing into it.
- **`TrapFrame`'s field order and the assembly must agree**, and no compiler can
  check that. The selftest does, on the machine: it plants a breakpoint and
  verifies the vector, error code, code selector, flags and stack pointer the
  handler saw are the ones the CPU pushed. Swapping two fields was tried, and
  the harness went red.
- The dispatcher reads its handler from an `AtomicPtr` written before the table
  is loaded, so a trap that reaches it has, by construction, a handler to call.
- Not handled yet, each with its first caller still to come: `swapgs` (needs
  ring 3), `XSAVE` (needs a scheduler; the kernel is built `+soft-float` and
  cannot disturb SSE state), and nested-interrupt policy (needs interrupts on).

---

## D-30 Descriptor encoding is a host-tested library — Status: Accepted (2026-08-13)

**Context.** `005-code-conventions.md` names "descriptor encoding" as a future
`libs/` crate. Phase 2 is where it arrives.

**Decision.** `libs/x86-tables` holds the GDT, IDT and TSS bit layouts, the
selector arithmetic, and the exception name and error-code tables. `wost` builds
descriptors from it and does nothing to them but load them.

**Rationale.** A wrong bit in a descriptor does not produce a diagnosable
failure; it produces a triple fault, or worse, a machine that runs with the
wrong privilege. The only place that can be checked is a host test against the
manual — `size_of::<Gate>() == 16`, the TSS's field offsets, the code descriptor
equalling `0x00209A0000000000`, and the error-code mask agreeing with a
transcribed copy of SDM table 6-1. Twenty-six such tests exist, and none of them
could run in the kernel.

**Consequences.** The crate is x86-specific, which is a first for `libs/`; a
port adds a sibling rather than growing this one. `ERROR_CODE_VECTORS` is
consumed both by the host tests and, as a `const` operand, by the assembler that
generates the stubs — so the two cannot drift.

---

## D-31 A fault is terminal and reported; a breakpoint resumes — Status: Accepted (2026-08-13)

**Context.** With an IDT installed, something has to decide what a fault means.

**Decision.** Vector 3 records its frame and returns. Every other vector panics
with the vector's name, the error code, and `RIP`/`CS`/`RFLAGS`/`RSP` — plus
`CR2` for a page fault, where the address is the whole of the report. Nothing is
recovered and nothing is retried.

**Rationale.** D-14: containment is supervised restart, and there is no
supervisor. A fault in ring 0 is a kernel bug, and the useful response is the
report. When user threads exist, this grows one branch — "was this ring 3?" —
in the one place faults arrive.

**Consequences.** The handler logs nothing. Taking a console lock in trap
context deadlocks against interrupted code that already holds it, which is the
hazard `001-boot.md` records against `SpinLock`; the panic path is exempt only
because it force-unlocks the consoles first. Breakpoints are recorded into
atomics for the same reason. The recording is unconditional — a trap handler is
no place for conditional compilation — while the accessor that reads it is
behind the `selftest` feature.

---

## D-32 The CPU protection baseline goes on at boot — Status: Accepted (2026-08-13)

**Context.** D-12 requires SMEP, SMAP, UMIP and NX from day one rather than as a
retrofit. Phase 2 is the first phase with a CPU to configure.

**Decision.** After the interrupt table is installed, the kernel enables every
one of the four that `CPUID` reports, then reads the control registers back and
logs what is actually on. Enabling is a safe call: none of these bits can
produce undefined behaviour, only a trap, and by then a trap is reportable.

**Rationale.** Ordering is the whole design. Enabling SMEP and SMAP against page
tables the bootloader built is a real risk — if Limine had mapped the kernel or
the framebuffer user-accessible, the next instruction fetch or the next pixel
would fault. Installing the IDT first turns that from a silent reset into a page
fault with an address. (It does not: verified 2026-08-13, all four enabled and
the framebuffer console still draws.)

**Consequences.**

- **QEMU is now `-cpu max`, not `-cpu qemu64`.** The default model exposes none
  of SMEP, SMAP or UMIP, so the kernel would have switched its baseline on
  against a CPU that cannot enforce it and the check that it did would have
  passed vacuously — which is exactly what the first run showed
  (`protections nx`). The VMware target and the real machine have all four.
- W^X, the other half of D-12, is a property of mappings and waits for Phase 3.
  Enabling NX now is what makes it enforceable then.
- `AC`, the flag that lifts SMAP for a deliberate user access, gets no accessor.
  It arrives with the checked user copy it exists for, not before.

---

## D-33 x2APIC only, because xAPIC needs page tables — Status: Accepted (2026-08-14)

**Context.** The local APIC has two encodings of the same registers. xAPIC is a
page of memory-mapped words at the physical address `IA32_APIC_BASE` reports;
x2APIC puts the same registers in model-specific registers. Phase 2 needs one
of them, and Phase 2 does not own any page tables.

**Decision.** x2APIC. `apic::init` requires `CPUID.01H:ECX.x2APIC`, switches the
processor into extended mode, and drives everything through `rdmsr`/`wrmsr`. A
processor without it is reported as `ApicError::NoExtendedMode` and the kernel
runs on with interrupts off.

**Alternatives.**

- *xAPIC through Limine's direct map.* Implemented first, and it page-faulted
  on the first register read. Under base revision 3 the direct map covers the
  regions the memory map lists, and a device hole is not one of them. (The
  fault printed `RIP` and a faulting address instead of resetting the machine,
  which is slice 1 doing exactly what it was built for.)
- *Map the page ourselves.* Needs a frame for the missing page-table levels,
  which needs an allocator. That is Phase 3, and pulling it forward would make
  the CPU phase a memory phase.
- *Fall back to Limine base revision 2*, which maps the first 4 GiB. Downgrading
  the bootloader contract to route around a gap we are about to close anyway.
- *Support both encodings.* The xAPIC half would be unreachable code until
  Phase 3 — speculative API with a hardware contract attached.

**Rationale.** x2APIC is strictly less machinery here, not more: no direct map,
no volatile memory access, and no argument needed about aliasing a device page
between a configuration write and an end-of-interrupt from a trap handler. It
has been in every Intel processor since Nehalem and every AMD one since
Bulldozer, it is what SMP will want regardless, and this project does not target
older hardware.

**Consequences.**

- Two writes to `IA32_APIC_BASE`, in order. The architecture has no transition
  from disabled straight to extended mode; setting both bits at once faults.
  `Base::x2apic` encodes that, and both writes are read back.
- Registers are still named by their xAPIC offset — that is how SDM table 11-1
  names them — and `Register::msr` derives `0x800 + offset / 16`. Both columns
  of the table are transcribed into the host tests.
- The APIC identifier is the whole register in this mode, not its top byte.
- `BootInfo::direct_map` was written and then deleted: with no caller left it is
  speculative API, and the 4 GiB bound it enforced was false besides. Phase 3
  adds it back with the code that can use it.

---

## D-34 A spin lock holds the interrupt flag — Status: Accepted (2026-08-14)

**Context.** `001-boot.md` recorded the hazard when `SpinLock` was written: a
lock taken in ordinary kernel context and also taken from an interrupt handler
deadlocks, and on a single CPU that is a certainty rather than a race. Nothing
could reach it while interrupts were off. Slice 2 turns them on.

**Decision.** Acquiring a `SpinLock` clears `IF`; the guard restores what it
found on drop. Every lock in the kernel, not a second lock type.

**Rationale.** The fix belongs in the lock rather than at the call sites, which
is what the original note said. Restoring rather than enabling is what makes
nesting correct: an inner lock that saw the flag already clear puts back a clear
flag, so it cannot hand interrupts back while an outer section still holds one.
Release order is part of it — unlock, then restore, because the other order
opens a window in which an interrupt arrives at code still holding the lock.

Asterinas parameterises its lock over the guard discipline
(`LocalIrqDisabled`, `PreemptDisabled`). That distinction earns its keep once
there is a scheduler; until then it would be a type parameter with one useful
value.

**Consequences.**

- The surviving hazards are the entries `IF` does not mask: a fault can be
  raised while a lock is held, and an NMI can arrive at any time. So D-31's rule
  that a trap handler must not log still stands — for a narrower reason.
- Interrupt latency is now bounded by the longest critical section. Scrolling
  the framebuffer console holds a lock across a screenful of pixel copies.
  Acceptable with no scheduler; revisit with one.
- `force_unlock` leaves the flag alone: obtaining a `PanicToken` cleared it, and
  the machine is not going to need it back.

---

## D-35 Enabling interrupts is safe because both preconditions are checked — Status: Accepted (2026-08-14)

**Context.** `enable_interrupts` was written in Phase 1 and deleted unused, on
the grounds recorded in `005`: an `unsafe fn` whose contract can only be stated
in prose is a guess. But the kernel crate carries `#![deny(unsafe_code)]`, so
whatever replaced it had to be callable from safe code.

**Decision.** `interrupt::enable` is a safe function that asserts its
preconditions: the interrupt table is loaded, and the legacy controller has been
silenced. It lives in `interrupt.rs`, not `cpu.rs`, next to the table that is
the reason it is safe. A private `set_flag` serves both it and
`InterruptState::restore`.

**Rationale.** `005` requires that a `wost` abstraction make the unsafe
operation underneath it *checkable*, not merely wrapped — the rule behind
`MmioRegion`'s bounds checks, `port::claim`'s registry, and `force_unlock`'s
`PanicToken`. This is the same rule applied to an instruction. Both
preconditions are already values something can read: `table_is_loaded()` asks
the CPU, and `pic::is_silenced()` asks the driver. The third — that locks are
IRQ-safe — is a property of the code rather than a runtime state, and is D-34.

The split between `enable` and `set_flag` is the design in miniature: setting
the flag the first time needs an argument, and putting back a flag that was
already set does not.

**Consequences.**

- `cpu::disable_interrupts` moved to `interrupt::disable`, which now returns the
  previous state so a critical section can restore rather than guess.
- `cpu.rs` and `interrupt.rs` reference each other. Harmless within a crate, and
  the alternative was `cpu` depending on the 8259 driver.
- Both asserts are reachable on a real machine — a kernel that cannot silence
  the 8259s or bring up the APIC declines to enable interrupts and says so,
  rather than reaching an assert.

---

## D-36 The timer is calibrated against the 8254, and ticks at 100 Hz — Status: Accepted (2026-08-20)

**Context.** The APIC timer counts the bus clock divided by a configurable
factor, and nothing reports that clock's rate. Phase 2 owns no page tables, so
every memory-mapped clock is behind the wall D-33 hit. The slice's
deliverables — a periodic tick and an uptime — both need the rate.

**Options.**

- *CPUID leaf 15H, or the hypervisor leaves.* Absent or zero under QEMU's
  software emulation; not trustworthy under VMware. A calibration source that
  works on neither test target is not one.
- *The HPET.* Memory-mapped at a device hole the bootloader does not map —
  D-33's constraint verbatim.
- *TSC-deadline mode.* Relocates the same problem: the TSC's rate is reported
  by the same leaves that are absent, so it would need the same 8254
  measurement first, plus invariant-TSC reasoning on top.
- *The 8254 over port I/O.* Present in every PC and every hypervisor, its
  crystal fixed at 1.193182 MHz since 1981, and ports are already safe to
  drive from the kernel.
- *Go tickless now.* D-15's destination, but a tickless kernel decides when
  next to wake, and that decision belongs to a scheduler that does not exist.

**Decision.** The 8254: counter 0, mode 0, one twenty-millisecond window,
bracketed symmetrically, divide-by-one, rounded to nearest once from the raw
pair. The result must land between 1 MHz and 50 GHz, with three attempts
before the kernel boots without a clock — and a timer failure vetoes nothing
else. The tick is periodic at 100 Hz; uptime interpolates the countdown
between ticks, sandwich-read and clamped monotonic. Calibration runs between
`apic::init` and `interrupt::enable`, unpreemptible by ordering alone. The
kernel's idle becomes halt–work–halt.

**Rationale.** The window sits between two hard bounds: the sixteen-bit
counter wraps ambiguously at 54.9 ms, and per-access jitter grows as the
window shrinks; 20 ms costs parts in ten thousand and keeps a 35 ms stall
margin. Channel 0 over the channel-2 speaker trick because its gate is always
high and its one output edge lands on a controller slice 2 masked — no port
`0x61`, no second device. The divide factor never enters the correctness
path: the count is derived from the same divided clock the window measured,
so the tick rate survives even a misfired divide write.

**Consequences.**

- The 8254 is programmed once and **parked**: a control word with no count
  halts it, and the permanent port claim means nothing can ever start it
  again. IRQ 0 is never unmasked, which also sidesteps every hypervisor's
  tick-coalescing machinery.
- Accuracy is one window's worth — roughly a part in a thousand — which
  affects only long-run wall-clock drift that nothing yet asserts.
- A critical section longer than a tick can swallow one (the APIC holds a
  single pending interrupt), and uptime undercounts by 10 ms. Accepted under
  D-34's recorded latency cost; Phase 4's tickless design is the fix.
- 100 Hz and periodic mode are explicitly disposable when D-15 lands; the
  vision doc counts wakeups as a feature, and this spends a hundred a second
  until a scheduler can spend fewer.
- `wost` gained five timer functions and zero new `unsafe` blocks: everything
  rides the MSR helpers whose gate D-33 built.

**Amended 2026-08-20**, after the phase's adversarial audit, in which two
independent reviewers converged on the same hole: the believability bounds
cannot catch a stall that crosses the counter's 54.9 ms span mid-window,
because the fold inflates the rate by a factor of two to four — inside
bounds that must span both targets' clocks. The window is now guarded in
depth: no gap between counter observations may exceed ten milliseconds, the
finished window must land within one such gap of its target, and two
consecutive windows must agree to one part in sixty-four — no single guard
survives a stall of exactly the wrong length, but no stall forges the same
wrong answer twice. A fourth guard came from the datasheet: the 8254 loads
a written count only on the crystal edge after its high byte, so the
baseline read now waits on the read-back status byte's null-count flag —
instant under both hypervisors, real on real hardware. Attempts rose from
three to five, giving the agreeing pair room around discarded windows.

---

## D-37 Keyboard input is polled until the I/O APIC exists — Status: Accepted (2026-08-20)

**Context.** The i8042 works, but its interrupt has nowhere legal to go.
IRQ 1's two routes to the CPU are the legacy 8259 pair — silenced in slice 2,
an invariant `interrupt::enable` asserts and a self-test pins — and the I/O
APIC, a memory-mapped device page behind the same missing page tables that
forced x2APIC (D-33). Slice 3 just gave the kernel a 100 Hz tick and an idle
loop that wakes on it.

**Options.**

- *Unmask IRQ 1 on the 8259s.* Interrupt-driven input today, at the cost of
  reopening the controller slice 2 existed to close: the `is_silenced`
  precondition and its self-test both weaken, a PIC-specific end-of-interrupt
  and spurious-IRQ-7 path get written, and all of it is deleted when the I/O
  APIC arrives.
- *Bring up the I/O APIC now.* The right final routing, but it needs a mapped
  device page, which needs page tables, which is Phase 3's entire subject —
  the CPU phase would become a memory phase, the exact trade D-33 refused.
- *Poll from the tick-woken idle loop.* No interrupt routing at all: the
  controller is configured to raise nothing, and each tick's wake drains it.
- *Defer the keyboard to Phase 3 wholesale.* Forfeits the phase's visible
  definition-of-done moment for no architectural gain — the driver and
  decoder are identical either way.

**Decision.** Poll. The i8042's configuration byte has both interrupt enables
cleared — the choice is enforced at the device, not assumed — and translation
cleared with them, so the decoder receives raw set 2. The idle loop drains
every pending byte per wake, batched eight events per lock hold, delivering
outside the lock.

**Rationale.** Latency is bounded by one tick: ten milliseconds against a
human perception threshold several times that, and against a keyboard that
buffers an order of magnitude more than a tick of typing. Nothing is thrown
away later — when Phase 3 pages the I/O APIC in, the polled call site in the
idle loop retires and the driver, decoder, and delivery path carry over
unchanged. It is the same deferral shape as D-33: the modern mechanism waits
for page tables, the phase ships on what is reachable.

**Consequences.**

- The driver lives entirely outside `wost`, like the UART and PIT drivers
  before it — ports have been safe since the claim registry — and the slice
  adds zero new `unsafe`.
- Input is dead if the timer declined: no tick, no wake, no drain. Accepted —
  the self-test would already be red on the timer checks, and a machine whose
  PIT and APIC both lie has larger problems than typing.
- Echoed characters ride the console lock from mainline context only; the
  trap-context prohibition of D-31/D-34 is never in play.
- Backspace echoes nothing until the console grows a cursor-retreat
  primitive with a real second caller (Phase 7's shell).

---

## D-38 The harness can press a key — Status: Accepted (2026-08-20; extends D-28)

**Context.** D-28's self-test can only observe what the machine does to
itself: it proves the interrupt path with a self-interrupt and the timer with
its own tick, but an input device's defining behaviour starts outside the
machine. Without an outside hand, the whole keyboard path — controller,
buffering, decoding — ships with its main road untested.

**Options.**

- *Host tests plus human eyes.* Leaves the only hardware-facing half
  unasserted; regressions surface as "typing stopped working" on a VMware
  boot, days later.
- *QEMU's monitor `sendkey` via `human-monitor-command`.* Works, but the
  release is automatic on QEMU's hold-time schedule — the check wants to
  assert the break code's arrival and order, so the timing must be ours.
- *A QMP crate.* A dependency for three fixed message exchanges, against
  D-25's precedent that a hand-rolled hundred lines beats a supply-chain
  edge.
- *A hand-rolled QMP client sending `input-send-event`.* Chosen.

**Decision.** `cargo xtask test-boot` binds a loopback `TcpListener` on an
ephemeral port before spawning QEMU and passes `-qmp tcp:{addr}` so QEMU
dials out — no fixed port to collide, no bind-then-connect race, no retry
loop. Serial is piped through a forwarding thread (every line still reaches
the console; a dying kernel's unterminated last line is flushed too) which
watches for one marker the kernel prints immediately before its bounded
wait: `selftest: awaiting injected keystroke`, a two-file contract kept in
step by comment exactly as the exit codes already are. On the marker, the
harness sends one command carrying an explicit press and release of `A`; the
kernel's check requires the make and then the break to decode, in order.

**Rationale.** Marker-gating rather than spraying keys from power-on: early
keystrokes land in firmware setup prompts or the bootloader menu, and the
self-test's own controller commands (config readback, echo, set readback)
must never interleave with injected bytes — the ordering is load-bearing and
commented at the check. The verdict stays the kernel's: any QMP failure
prints its cause and retires the injector, whereupon the kernel's doubly
bounded wait (a tick budget, plus an iteration cap so a dead timer cannot
convert red into hang) fails the check, and the sixty-second kill stands
behind everything. Reply handling is token classification, not JSON parsing,
which is sufficient because both commands are fixed strings, the response
set is tiny, and unsolicited `SHUTDOWN` events — which genuinely arrive on
this socket when the exit device fires — need only to be skipped.

**Consequences.**

- `test-boot`'s serial output now passes through a pipe; behaviour is
  preserved line-for-line, and this is the note recording that it is
  deliberate.
- Injection is unconditionally on for `test-boot` — a check that only
  sometimes receives its stimulus is a check that only sometimes can fail.
- The QMP failure path was broken once on purpose (the `-qmp` argument
  deleted): the run went red on the keystroke check rather than hanging.
- On real hardware there is no QMP and no injector; the same check would go
  red, which is correct — nothing outside presses a key there either. The
  self-test feature never ships in an image (D-28), so this changes nothing
  for a real boot.

---

## D-39 A port claim covers the width it drives — Status: Accepted (2026-08-22)

**Context.** An adversarial audit of Phase 2 found `wost::arch::x86_64::port`
recording one bit per claim whatever the access width. `port::claim::<u32>`
authorises an `out dx, eax`, which drives four consecutive ports; the registry
believed it owned one. `kernel/src/arch/x86_64/debug_exit.rs` is a live
instance: it claims `Port<u32>` at `0xF4` and the harness gives QEMU
`iosize=0x04`, so the kernel drives `0xF4..0xF8` against a one-port claim.
`PortBlock` had the mirror of the same hole — `port_for` asserted
`offset < N`, so a `u32` access at `N - 1` ran three ports past the block into
whatever answered next.

Nothing collided in practice; every `PortBlock` caller happens to use `u8`.
But `005-code-conventions.md` states that what `wost` owes its callers is that
every abstraction make the unsafe operation underneath it *checkable* — and
names `port::claim` as the worked example, "so exclusivity is verified rather
than promised". It was promised.

**Options.** (a) Leave it and document the caveat; (b) widen the claim and the
bounds check in place; (c) widen them, and move the bookkeeping somewhere it
can be tested.

**Decision.** (c). `size_of::<T>()` is the number of ports an access drives —
which the sealed `PortValue` trait makes true by construction, since its only
implementors are the primitives of exactly those widths. `claim` reserves that
many; `PortBlock::read`/`write` check `offset + size_of::<T>() <= N`. The
bitmap itself moved to a new `libs/port-space` crate, on D-30's precedent: it
is arithmetic over bits with no hardware behind it, so it belongs where
`cargo test` can reach it.

`Port::new` and `PortBlock::new` became private in the same change. They were
`pub unsafe`, which left a documented way to construct a handle the registry
knew nothing about; `claim` and `claim_block` are now the only doors.

**Rationale.** The width is not a detail a caller should have to remember —
it is already in the type. Extracting the bitmap bought thirteen host tests
including a sweep of every way two spans can meet and a pass over all 65536
ports, none of which a `no_std` cross-compiled module could have carried.

**Consequences.** One on-machine check joins them (`a claim covers the width
it drives`): a 32-bit claim two ports below COM1 must be refused, and a byte
claim at the same port must then succeed — which proves both that the port
was free, so the refusal was about the width, and that the refused claim
recorded nothing. Broken once by making `reserve` ignore its length, and the
boot went red. Phase 3's device mappings are to inherit this discipline, not
the old one: `007-memory.md` asks for "a claim discipline like `port::claim`'s"
and that now means covering the span an access reaches.

---

## D-40 The linked image is verified, not assumed — Status: Accepted (2026-08-22)

**Context.** The same audit read the built ELF back and found `.got` placed
*after* `.bss`. It linked, it booted, and it was wrong: `linker.ld` named
neither `.got` nor `.got.plt`, so the linker placed them by orphan heuristic,
and a loadable section following a `NOBITS` one forces the whole of `.bss`
into the file. 29048 bytes of zeros were being written into every image, and
`.bss` was no longer being zeroed by the bootloader but loaded from disk.

A linker script fails loudly when it is malformed and quietly when it is
merely incomplete. Nothing in the gate could tell the difference.

**Options.** (a) Name the orphans and move on; (b) name them and check the
result; (c) adopt a linker-script linter or an external `readelf` in the gate.

**Decision.** (b). `linker.ld` now names `.got`, `.got.plt` and
`.data.rel.ro`, puts `.bss` last and `(NOLOAD)`, discards `.comment`, and
gives the Limine request block its own `KEEP()`-ed `.requests` section.
`xtask/src/elf.rs` reads the linked ELF after every build and requires: the
entry point in the higher half; every allocated section inside a `PT_LOAD`;
no segment both writable and executable; no `NOBITS` section costing file
space; and `.text` and `.requests` present and loaded. External tooling was
rejected on D-16's grounds — the build stays self-contained, and this is
~200 lines of header reading.

**Rationale.** W^X at the segment level was already true and already load-
bearing (Limine maps each `PT_LOAD` by its flags), but nothing asserted it,
and Phase 3 is about to build page tables that must reproduce it. An orphan
section is the failure mode that survives a link, boots, and shows up as
something else entirely months later.

**Consequences.** The check found a real gap on its first run: the request
block had no section of its own — it was folded into `.data` — so nothing
could confirm the block `wost::entry!` emits had linked at all. Giving it a
name also let it be `KEEP()`-ed, which is what D-21's note about `#[used]`
statics being dropped by a linker actually asks for. Broken once by placing a
loadable section after `.bss`; the build went red naming `.bss` and the 29048
bytes. Thirteen host tests cover the policy and the reader, including a
truncated-file sweep, because a verifier that panics on a malformed input is
a verifier that stops the build for the wrong reason.

---

## D-41 Three authorities for `unsafe`, and doctests where they can run — Status: Accepted (2026-08-22)

**Context.** `005-code-conventions.md` admitted code to `wost` on a judgement:
"an operation that cannot be expressed safely". That phrasing has held, but it
is not a test a reviewer can apply to a borderline case, and Phase 3 is about
to add more `unsafe` to `wost` than any phase since the split — frame
ownership, page-table mutation, a heap.

The second research pass (`008-research-notes-2.md` §2) surfaced two things
`005` did not have. Microsoft's Pragmatic Rust Guidelines publish a **closed
list** of valid reasons to write `unsafe`, which turns the judgement into a
checklist. And the Rustonomicon states the standard a wrapper is actually held
to — *safe code must not be able to cause UB through this abstraction, however
it is called* — which is what `005`'s "checkable, not merely wrapped" was
reaching for.

The same pass found `005` overshooting in the other direction. It rejects
C-EXAMPLE outright, reasoning that "these crates are `no_std` and
cross-compiled, so doctests cannot run". True of `wost` and `kernel`. False of
`libs/`, which carry `#![cfg_attr(not(test), no_std)]` **precisely so** they
build for the host under `cargo test` — where doctests compile and run, inside
the bare `cargo test` that `default-members` already scopes and `cargo xtask
ci` already gates.

**Options.** (a) Leave the admission test as a judgement; (b) adopt the
taxonomy as guidance in prose; (c) adopt it as the admission test, name the
Nomicon as the standard, and split C-EXAMPLE by where doctests execute.

**Decision.** (c). Three reasons admit `unsafe` to `wost` — a novel
abstraction, a measured performance need, or FFI/the platform — and nothing
else. Every export is held to the sound-wrapper principle. The Unsafe Code
Guidelines Reference is explicitly *not* an authority, because it describes
itself as advisory and partly historical, and citing it as a rule has misled
people before.

C-EXAMPLE is adopted in `libs/` only. `wost` and `kernel` keep the exemption
on the same grounds as before, now stated as the fact it is rather than a
blanket preference.

**Rationale.** The taxonomy's value is that reason 2 is falsifiable: it needs
a number, and nothing in this kernel has ever produced one, which is itself
worth knowing. The C-EXAMPLE split costs nothing — the harness that runs those
doctests has existed since D-26 — and it lands exactly where it is most useful,
on the encoding crates D-30 keeps producing, where an example is the clearest
statement of a bit layout available.

**Consequences.** No code change; `005` gains the taxonomy, the Nomicon
standard, and the split, plus the three rustdoc conventions that go with it
(first line under fifteen words, conventional section order, `#[doc(hidden)]`
for technically-public items). `libs/x86-paging` is the first crate written
under the new rule and carries doctests on its entry encoders. The lint that
would enforce example presence does not exist, so this one is enforced by
review — which is why it is written down.

---

## D-42 Limine base revision 4 — Status: Accepted (2026-08-22)

**Context.** The kernel asked for base revision 3, and it asked by accident:
`wost::entry!` emits `BaseRevision::new()`, and in the pinned `limine` 0.5.0
that constructor is `with_revision(3)`. No decision was ever taken.

That default has been shaping the design. `BootInfo`'s doc comment records the
direct map's absence as a consequence of it — "under base revision 3 it covers
only the regions the memory map lists — not the device holes" — and D-33
deleted the field for that reason. `007-memory.md` inherits the same sentence
as "device holes unmappable — the wall itself".

The protocol has since reached base revision 6, with 0–5 marked deprecated.
Reading the revision history rather than the deprecation notice is what
matters here:

| Revision | Change relevant to this kernel |
|---|---|
| 3 | HHDM covers Usable, Bootloader-reclaimable, Executable-and-modules, Framebuffer |
| **4** | HHDM **adds Reserved (Mapped), ACPI reclaimable, ACPI NVS**; adds the `RESERVED_MAPPED` memory-map type; **guarantees ACPI tables are mapped** |
| 5 | SMBIOS mapped; stricter x86-64 control-register handoff; more I/O APIC entries masked; IOMMUs disabled; local APIC left in a defined state on every processor |
| 6 | Drops identity mapping; register-state guarantees for aarch64, RISC-V, loongarch64 — nothing x86-64 |

**The obvious blocker is not one.** `001-boot.md` lists "Limine crate 0.6.x
when its nightly requirement lands on stable" as an open item, which reads like
the revision is gated behind it. It is not: `limine` 0.6.5 still carries
`#![feature(ptr_metadata)]` and fails to build on stable 1.97.1 — confirmed by
building it — so the `=0.5.0` pin stands, but the base revision is a *protocol*
parameter and 0.5.0 exposes `BaseRevision::with_revision(u64)` as a public
`const fn`. The crate version and the protocol revision are independent
choices, and only the first is blocked.

**Options.** (a) Stay on 3 and keep the wall; (b) take 4; (c) take 5 for the
cleaner handoff; (d) take 6.

**Decision.** (b), base revision 4, requested explicitly through
`with_revision` rather than inherited from `new()`.

**Rationale.** 4 is the revision that buys something this phase needs twice
over: it is what lets the HHDM reach a device hole at all, and it guarantees
the ACPI tables are mapped, which is how slice 5 finds the I/O APIC through
the MADT. 5 and 6 were declined for now on the no-speculative-change rule —
5's guarantees are about state this kernel already establishes for itself
(it masks the legacy controllers and programs the local APIC in
`start_interrupts`), and 6 is entirely non-x86-64. Both remain available as a
one-argument change, which is the point of taking the decision explicitly.

**Consequences.** `entry!` asks for 4; `is_supported()` is already checked and
asserted in `kernel_main`, so a bootloader that will not honour it stops the
boot with a message rather than misbehaving quietly. The vendored Limine v10
does honour it — verified by boot, which is the only way this could be
verified.

The memory map gains a type the pinned crate does not know
(`RESERVED_MAPPED`), so `limine_glue` maps entry types through an exhaustive
match with an explicit unknown arm and treats anything unrecognised as
reserved. That is the `#[non_exhaustive]` discipline `005` mandates for our own
error enums, applied to a foreign one — and it is what makes a future
revision bump a compile-time question rather than a silent misclassification.

Base revision 4 does **not** change the plan to map device MMIO through
`MmioRegion` with an explicit memory type. It removes the wall; it does not
remove the reason for the door, and D-43's direct-map contract still excludes
device holes deliberately.

---

## D-43 The frame allocator: mechanism in `wost`, policy in `libs/` — Status: Accepted (2026-08-22)

**Context.** `007-memory.md` holds two questions open together, and they turn
out to be one question. What shape should the physical frame allocator take —
bitmap, buddy, or staged? And where does the D-13 boundary fall, given that
table *mutation* is `wost`'s but allocator *policy* is arithmetic?

`008-research-notes-2.md` §3.1 found the answer already published. Asterinas
calls it **Safe Policy Injection**: the privileged framework keeps the
*mechanism* and the invariant — for frames, the conversion that makes "every
frame handed out came from unused memory" true — while the *policy* that
chooses which frame lives outside the trusted base in ordinary safe Rust.
Asterinas has exactly three such injection points, and the frame allocator is
one of them.

This project already has the pattern under a different name. D-39 moved the
I/O port bitmap to `libs/port-space` because it is arithmetic with no hardware
behind it, leaving `wost::port` holding the lock and the instruction, and
`005-code-conventions.md` records the rule that came out of it: *a bound stated
in prose is not a bound; where the checkable form needs arithmetic, the
arithmetic goes to a `libs/` crate.*

**Options.** (a) Allocator wholly in `wost`, bitmap and all; (b) allocator
wholly in `kernel`, with `wost` exposing raw physical writes; (c) split it —
`wost` owns the frame handle and the physical access, a `libs/` crate owns the
search.

**Decision.** (c), on the `port-space` precedent exactly.

`libs/frame-space` holds two bitmaps and knows nothing but frame indices:
`managed` (usable RAM under our control) and `free` (which of those are not
handed out). Its capacity is a const parameter, so the kernel names its ceiling
and tests use spaces small enough to sit on a stack. `wost::mm` wraps one in a
`SpinLock` and exports `Frame` and `Segment` — handles whose existence *is* the
claim and whose `Drop` releases it.

Shape: a bitmap with first-fit-from-a-rotating-hint, **order-aware from the
start**. `008` §3.1 argued that point and this phase proves it: the direct map
wants 2 MiB pages, so `allocate_contiguous(512, 512)` is needed inside Phase 3
rather than in Phase 8. A buddy remains the upgrade, and it is a change to one
`libs/` file because the API speaks in frames rather than in the structure that
finds them.

**Rationale.** Two bitmaps rather than one is the decision inside the decision.
One bitmap cannot distinguish a double free from a frame that was never ours,
and `free` is what a `Drop` impl calls — the single place a bug is silent.
With both, releasing a foreign or already-free frame is a refusal the caller
can see, which is what `005` means by *checkable, not merely wrapped*.

Nothing computes the kernel image's extent to exclude it. Only
`MemoryKind::Usable` is inserted, so the image, the bootloader's structures and
the ACPI tables are excluded **by the memory map itself** — including the
megabyte of bitmap, which is `.bss` inside the loaded image. A range this code
computed would be a second source of truth for something the bootloader already
knows.

The reclaimable kinds are deliberately *not* allocatable. Bootloader structures
are still being read when the allocator is built, and the ACPI tables have not
been parsed. Reclaiming either is a later decision with a precondition
attached, not a default.

**Consequences.** 20 host tests, including a run that crosses the boundary
between two bitmap words and a full drain-and-restore sweep that requires every
frame to be conserved. Six on-machine checks, and one of them earned its place
the hard way: the first version of "allocation returns a zeroed frame" passed
with the zeroing removed, because it short-circuited on an address comparison
and because the hint means a released frame is rarely the next one handed out.
Breaking it once — which D-28 requires and which nearly did not happen —
exposed a check that could not fail. It was replaced by
`dirty memory can be zeroed, and seen to be`, which writes `0xAA`, requires the
kernel to *see* it, zeroes, and requires the kernel to see that; breaking
`Frame::fill` turns the harness red. The composition `allocate_zeroed =
allocate + zero` rests on that check plus three lines of code, and the selftest
says so rather than pretending otherwise.

The ceiling is 16 GiB, costing two bits of `.bss` per frame — one megabyte,
which the image does not carry because both bitmaps start zeroed. The release
kernel is 510 KiB on disk with a megabyte of `.bss` behind it, and D-40's ELF
verifier confirms `.bss` stayed `NOBITS`. Memory above the ceiling is reported
as ignored at boot rather than silently lost.

Deliberately absent: the untyped-memory split (`UFrame`/`USegment`) that
`008` §3.2 and D-44 adopt as a discipline. It arrives with the first caller
that can map a frame into a second address space, which is Phase 5. Writing it
now would be a safety contract with nothing to check it against
(`005-code-conventions.md`).

---

## D-44 Untyped memory is a type discipline, not an accounting mechanism — Status: Accepted (2026-08-22)

**Context.** `002-architecture.md` has carried an open question since the first
research pass: *is seL4-style untyped-retype — callers donating memory for
kernel objects — worth adopting to kill kernel-memory DoS?* `007-memory.md`
made answering it a Phase 3 obligation, because the answer shapes every
kernel-object API after this one.

`008-research-notes-2.md` §3.2 found that the question conflates two different
ideas that share a word.

**seL4's untyped/retype is an accounting mechanism, and it is expensive in
three separate ways.** Each untyped capability carries a single watermark, moved
first to the new object's alignment and then past its size — so creating a
4 KiB object and then a 16 KiB one strands the 12 KiB between them until *both*
children are revoked. The community critique goes further: fixing the
allocation policy inside the kernel contradicts seL4's own principle of keeping
policy out of the kernel, because the kernel will refuse a gap that would
satisfy the alignment. And `003-research-notes.md` already recorded that
revocation is long-running and needs abortable sub-operations.

**Asterinas's untyped memory is a type discipline, and it carries.** Its
premise: safe Rust references cannot uphold their guarantees for *externally
modifiable* memory — user-mapped, DMA-capable, or MMIO — so such memory gets a
separate type admitting only plain-old-data copies. The consequence is
structural rather than procedural: the user address space accepts *only*
untyped frames, so mapping kernel memory into userspace is not a check that
could be forgotten but a signature that will not typecheck. The DMA
abstractions take the same argument, so no memory is DMA-accessible until a
driver asks for untyped memory specifically.

**Decision.** Reject seL4-style untyped-retype as the kernel-memory accounting
mechanism. Adopt Asterinas-style untyped memory as a type discipline, arriving
with its first real caller in Phase 5. Answer kernel-memory DoS at the handle
layer with per-process quota accounting.

**Rationale.** The quota is a *policy* decision and stays revisable; making
callers donate memory is an *ABI* decision and is permanent. Everything
untyped-retype buys can be had from a counter attached to a job, and the
fragmentation, the revocation machinery and the policy-in-kernel objection all
go away with it.

The discipline is the half that must be early, because it is the half that
cannot be retrofitted — which is `002` pillar 5's whole argument and
Asterinas's own stated lesson. It costs nothing today: `wost::mm` hands out
`Frame` and `Segment`, and the untyped variants join them when something can
map a frame into an address space that is not the kernel's.

**Consequences.** `002-architecture.md`'s open question is closed, and the
naming collision is recorded here so it is not re-litigated: "untyped" in this
kernel means *externally modifiable, POD-copy only*, never *donated for kernel
object creation*. Phase 5's handle-table doc inherits the quota obligation.
`wost::mm`'s module documentation names the absent split and why it is absent.

---

## D-45 The kernel's own tables, and a `CR3` switch that is a safe function — Status: Accepted (2026-08-22)

**Context.** `007-memory.md` names the switch to the kernel's own page tables
as the phase's one irreversible moment: *"the switch is the one moment a
mistake reports as a triple fault rather than a page fault"*. At that
instruction the kernel is executing from, logging through, and standing on
mappings the bootloader provided. A table missing any of them faults on an
instruction fetch with no handler mapped, and the machine resets with nothing
on the wire.

That is also the sharpest possible test of what `005-code-conventions.md`
demands of `wost`: every abstraction must make the unsafe operation underneath
it *checkable*, not merely wrapped.

**Options for the switch.** (a) `unsafe fn activate` with a `# Safety` section
listing what the caller must have mapped; (b) the same, plus a `debug_assert`;
(c) a **safe** `activate` that walks the new tables for the addresses whose
absence would be fatal, and refuses rather than loading.

**Decision.** (c). Before `CR3` is written, `activate` translates — *in the new
tables* — the address of its own code and the current stack pointer, plus a
page of headroom below the stack for the first push after the switch. Code
must come back executable, stack writable. A table failing either check is
refused with the old one still loaded, so the failure is a `Result` on the
serial log instead of a reset.

`activate` takes the space **by value**. That is what makes
`AddressSpace::map` safe: nothing can add a mapping to a table the CPU is
already walking, because after the switch the caller has nothing left to map
into. A second call is refused with `AlreadyActive`, asked before the walks so
the error names the real problem rather than whichever mapping the second
caller's empty table happened to lack.

**The direct map's contract.** `008-research-notes-2.md` §3.3 proposed *usable
RAM only, device holes excluded*. Half of that survived contact.

Excluding device holes is right and is kept: `Reserved` is where MMIO lives,
and mapping it here would give a device the direct map's cache type instead of
the one it needs. Devices are mapped deliberately, through `MmioRegion`, and
are never reachable by accident.

*Usable RAM only* was wrong, and would have triple-faulted on the first switch.
The kernel is executing on a stack the bootloader placed in
`BootloaderReclaimable` and logging through a `Framebuffer` region. The direct
map therefore covers everything that **is memory** — `Usable`,
`BootloaderReclaimable`, `KernelAndModules`, `Framebuffer`, `AcpiReclaimable`,
`AcpiNvs` — and nothing that is not. It is built at the offset the bootloader
already used, which is what keeps every pointer the kernel is holding valid
across the switch rather than requiring a sweep to rewrite them.

2 MiB pages wherever the addresses and remaining length permit: 511 MiB of
direct map costs a few hundred kilobytes of tables instead of a megabyte, and
one TLB entry covers 2 MiB instead of 4 KiB.

**W^X, the D-12 debt owed since boot.** The linker script gained six symbols
bounding the three protection classes, so the image is mapped by *what each
part may do* rather than as one range: text read-only and executable, rodata
read-only and not, data writable and not. There is no fourth combination,
which is the whole of W^X. Intermediate entries are deliberately permissive
(`PRESENT | WRITABLE`, never `NO_EXECUTE`) because permissions intersect down a
walk — a restrictive parent would silently cap every leaf beneath it, so the
enforcement is at the leaves, which is where the check reads it back.

**Rationale.** The check is cheap — two walks of four levels — and it converts
the single most dangerous operation in the kernel from a documented obligation
into a verified one. `005` says a bound stated in prose is not a bound; for
`CR3` the prose would have been three paragraphs of `# Safety` that no caller
could confirm it had met.

**Consequences.** Four on-machine checks. `the loaded page table is ours`
requires both that the switch was attempted and that `CR3` holds the root we
recorded — a kernel that noted success and kept the old tables satisfies only
the first. `the kernel image obeys write-xor-execute` walks the live tables at
three points per section, because a run of large pages can be right at the
edges and wrong between them; broken once by mapping `.data` executable, and
the harness went red. `the direct map is writable and never executable` covers
the largest writable mapping in the kernel. `a second address space is refused`
is the property that makes `map` safe.

Two bugs the checks caught, both in the checks themselves. The W^X probe
computed its midpoint as `(start + last) / 2`, which overflows on a pair of
higher-half addresses; it now works from offsets. And `activate` validated
before testing the latch, so the second-switch check saw `CodeUnmapped` rather
than `AlreadyActive` — correct behaviour, useless diagnosis.

Recorded for SMP: a direct map means unmapping anything from it needs a TLB
shootdown — an IPI to every CPU. Free while single-core, and cheaper to write
down now than to discover in Phase 12.

---

## D-46 KASLR waits, and this is what waiting costs — Status: Accepted (2026-08-22)

**Context.** D-12 put KASLR in the security baseline "on from the start, not
retrofitted". It has been owed since Phase 0 and blocked throughout on the same
thing: `linker.ld` hardcodes `0xffffffff80000000`, so the image is not
relocatable and `kaslr: yes` in `boot/limine.conf` would have nothing to
randomise. `007-memory.md` requires this phase to either take the relocation
work or record the deferral.

**Options.** (a) Make the image relocatable now — position-independent code,
a relocation pass at boot, and every hardcoded address in the linker script and
the mapping code reworked to be base-relative; (b) defer, and name the cost.

**Decision.** (b), deferred to Phase 12 alongside the other hardware-security
items, with the cost stated rather than left implicit.

**Rationale.** The relocation work is not large, but it is the wrong shape for
this phase. Slice 2 has just made the image's layout load-bearing in a new way
— six linker symbols now bound the protection classes, and the mapping code
turns each into a physical address through the bootloader's reported base.
Making all of that base-relative in the same phase that first depends on it
would mean changing the foundation and the thing standing on it together, and
the failure mode is a triple fault with nothing on the wire.

**What the deferral costs, so the next session need not rediscover it.** Every
phase that adds a mapping adds an assumption about where the kernel lives. Slice
2 added three. Phase 4's per-CPU stacks will add more, and Phase 5's userspace
will add the first mappings whose *absence* of randomisation is externally
observable. The work is strictly cheaper now than after those land, and the
argument for deferring gets weaker with each one — it is a debt with interest,
not a debt at par.

Two Phase 12 items assume the kernel controls its own layout and would be
taken in the same breath: CET shadow stacks, which are a per-stack allocation
decision, and LASS, whose whole premise is a hard user/kernel address split
(`008-research-notes-2.md` §6).

**Consequences.** D-12's KASLR line remains outstanding and is now the only
part of the security baseline still owed. `001-boot.md`'s known limitation
stands. W^X, NX, SMEP, SMAP, UMIP, guard pages and zero-on-allocate are all
done, so this is the last one — which is itself an argument for not letting it
drift much further.

---

## Cross-references

- `000-vision.md` — destination + phase roadmap
- `001-boot.md` — boot contract and disk layout (D-03, D-16)
- `002-architecture.md` — the native architecture (D-06..D-15)
- `003-research-notes.md` — evidence tiers and citations behind
  Verified statuses
- `005-code-conventions.md` — how the code is written (D-21, D-22, D-23)
- `006-cpu-fundamentals.md` — Phase 2: descriptor tables, the fault path, the
  CPU protection baseline, the interrupt controller, the calibrated tick, and
  the polled keyboard (D-29..D-38)
- `007-memory.md` — Phase 3 charter: the debts D-12, D-32, D-33 and D-37
  deferred here, the slice proposal, and the decisions still open (each
  becomes an ADR as it is taken)
- `008-research-notes-2.md` — the second research pass, opening Phase 3:
  what the stable toolchain now allows, the evidence behind the decisions
  `007` holds open, and the proposed additions to `005`. Its marks are its
  own; `003`'s tiers still govern what counts as Verified here

The 2026-08-22 adversarial audit closed the gaps recorded in D-39, D-40, and
the amendments to D-22 and D-25. Its two standing lessons, worth reading
before the next phase: a gate is only as wide as its configuration matrix,
and an abstraction that promises a bound must be given something that can
check it.
