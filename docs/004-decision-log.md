# 004 — Decision Log

Architecture Decision Records for The Wizard's OS. Each entry captures the
context, the options weighed, the decision, and its consequences. Statuses:
**Accepted** (settled), **Verified** (settled + supported by the
adversarially-verified research in `003-research-notes.md`), and
**Provisional** (settled and built upon, but resting on evidence that has not
been independently confirmed — carries a falsification condition, D-71).

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
ATC'25) demonstrates a Rust framekernel at real-system scale. Its containment
property is verified; its percentage-based TCB and broad performance headlines
are not evidence this project relies on, and only single-core syscall dispatch
is a defensible performance summary (D-71, `010` §1). Its lesson that safety
must be designed in clean-slate is exactly this project's position. C++ remains
welcome in userspace later.

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
Limine's boot protocol (base revision 4 since D-42); `limine.conf` +
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
process isolation of subsystems. The framekernel evidence supports containing
unsafe code inside a monolithic design; `010` §1 withdrew the percentage-based
TCB and broad performance claims while leaving that containment result intact.
Capsicum's measurements (unverified tier) found direct capability-checked
syscalls cost only a few percent — supporting "fast path over message passing."

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

**Amended by D-75.** NetSurf remains the integration target, but "browser" and
"web browsing" are historical shorthand. The current milestone is document-web
rendering and proof that libc, windowing, networking, and TLS compose.

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

**Rationale, amended 2026-08-23 by `010` §2.** The original entry contrasted
batching and cache locality with trap avoidance. FlexSC does not support that
contrast: batching reduces both direct mode-switch cost and indirect cache
cost. The larger result is structural asynchrony; eliminating the trap is a
smaller additional gain and can cost a dedicated core. Zircon's handle-centric
authorization remains verified as Fuchsia's deployed design; the policy escape
hatch mirrors Zircon Job policy (verified caveat). Capsicum independently
converged on capabilities + no global namespaces (unverified tier).

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

## D-08 Filesystem: custom WizFS, CoW from the start — Status: Provisional (2026-08-23; see D-73, which moves it out of Phase 6)

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

**Amended by D-73.** The bullets above preserve the founding decision. The
current plan strikes blind upserts as an ABI rule, rejects parity RAID because
its prerequisites are out of scope rather than because RAID-Z has a write hole,
and moves WizFS from Phase 6 to Phase 13.

---

## D-09 App formats: ELF64 + WASI 0.3 components as first-class — Status: Provisional (2026-08-23; restated by D-72 as wasip1-first)

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

## D-13 Framekernel discipline: all `unsafe` in one privileged crate — Status: Provisional (2026-08-23; the containment property is verified, the published numbers are unreplicated — see D-71 and `010` §1)

**Context.** Research pass, strongest verified result (Asterinas,
USENIX ATC'25).

**Decision.** The kernel is one address space with monolithic
performance, but ALL `unsafe` Rust lives in a small privileged framework
crate — **`wost`** (Wizard OS Standard, our OSTD analogue): boot,
paging, MMIO, port I/O, DMA, context switch, interrupt entry. Every
other kernel crate (scheduler, VFS, WizFS, network, drivers) is
`#![forbid(unsafe_code)]` safe Rust written against `wost`'s safe
abstractions.

**Rationale, amended by D-71 and `010` §1.** The paper reported a ~14% TCB ratio
and broad Linux-parity single-core results. Those numbers are retained here as
historical context, not as evidence: absolute TCB size is the relevant audit
cost, and only single-core syscall dispatch is a defensible performance
summary. No independent replication exists, which is why this ADR is
Provisional. The containment *property* is verified on this codebase, and the
authors' lesson that safe MMIO/port/DMA abstractions must exist from day one
still stands.

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
  (`profile = "minimal"`, `rustfmt`/`clippy`, the `x86_64-unknown-none`
  target). `rust-mingw` is deliberately absent from the component list; rustup
  supplies it with a Windows-GNU host, while naming it breaks other hosts
  (D-24).
  *Corrected 2026-08-12: this bullet previously read "stable (1.97.x), pinned"
  while the file actually said `channel = "stable"` — a floating channel. See
  D-24.*
- **`limine` crate = 0.5.0 exactly**: 0.6.x requires nightly
  (`ptr_metadata`). The pinned crate requests protocol base revision 4 through
  `with_revision` (D-42), confirmed against the vendored Limine v10 binary by
  booting QEMU and VMware. Revisit the crate pin when 0.6's requirement lands
  on stable or if we adopt nightly; crate and protocol revisions are separate.
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

**Consequences.** Two firmware and framebuffer paths are exercised
continuously: EDK2/QEMU and VMware EFI/SVGA-II. NIC, GPU, and storage drivers
do not yet exist, so their two-target obligations begin with their own phases;
VMware has no virtio, which is why Phase 6 chooses NVMe for storage (`010`
§10). GDB-stub debugging is an open item (no native Windows gdb yet).

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

## D-20 Networking plan: smoltcp in-kernel, rustls in userspace — Status: Accepted (planned)

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
during boot looks like from outside. **At adoption**, six checks existed —
serial and console bring-up, framebuffer geometry, non-overlapping colour
masks, port-claim exclusivity, and lock exclusion. That total is a historical
snapshot; the latest phase close records the current total. The harness was
verified by breaking one and confirming it goes red.

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

## D-31 A fault is terminal and reported; a breakpoint resumes — Status: Accepted (2026-08-13) (amended by D-77; narrowed to kernel faults by D-102)

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
hazard `001-boot.md` records against `SpinLock`; the terminal panic path tries
reporters without blocking instead of invalidating a live guard. Breakpoints
are recorded into atomics for the same reason. The recording is unconditional
— a trap handler is no place for conditional compilation — while the accessor
that reads it is behind the `selftest` feature.

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

## D-34 A spin lock holds the interrupt flag — Status: Accepted (2026-08-14) (amended by D-77)

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
- Recovery is private to `wost`'s non-returning panic boundary. Kernel panic
  reporting never waits for or forcibly releases a kernel lock; a busy output
  channel may be omitted.

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
`MmioRegion`'s bounds checks, `port::claim`'s registry, and the private terminal
panic boundary. This is the same rule applied to an instruction. Both
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

**Superseded in part by D-57 (2026-08-23).** The tick, the interpolated
uptime and the monotonic clamp are gone; what survives is the measurement —
the same window, the same guards, the same agreeing pair — now applied to two
clocks instead of one. The two debts this recorded are discharged there.

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

## D-45 The kernel's own tables, and a `CR3` switch that is a safe function — Status: Accepted (2026-08-22) (amended by D-76; its one-way mapping retired by D-86)

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

**Consequences.** D-12's KASLR line remains outstanding and is the only
baseline item deferred as far as Phase 12. `001-boot.md`'s known limitation
stands. W^X, NX, SMEP, SMAP, UMIP, zero-on-allocate, and ordinary thread-stack
guard pages (D-52) are done. The boot and double-fault IST stacks remain a
separate Phase-5 guard-page debt, so KASLR is not literally the only unfinished
line of D-12.

---

## D-47 The kernel heap lives in the direct map — Status: Accepted (2026-08-22) (the `HEAP` → `FRAMES` order it established is retired by D-85)

**Context.** `007-memory.md` frames the heap decision as hand-rolled versus a
vetted crate — "the first real test of the dependency policy against
`alloc`-ecosystem convention". `008-research-notes-2.md` §3.5 surveyed `talc`,
`good_memory_allocator`, `slab_allocator_rs` and `linked_list_allocator` and
concluded that the crate is the wrong question: what matters is the boundary,
because Asterinas's answer is a set of type conversions (unused page → slab →
free slot → object) rather than a search algorithm.

D-45 then made the question sharper in a way neither anticipated. `activate`
consumes the address space so that nothing can map into a table the CPU is
already walking — that is what makes `AddressSpace::map` a safe function. A
heap that grew by mapping fresh pages would have to undo it.

**Options.** (a) Reserve a fixed virtual heap range while the tables are being
built, and bump within it; (b) reopen mapping after activation behind a second
guarded API; (c) put the heap in the direct map, which is already mapped
writable and never executable.

**Decision.** (c). The heap takes frames from the frame allocator and addresses
them through the direct map. No page-table entry is ever added for it, so
D-45's one-way property is untouched, and the heap grows to whatever the
machine has rather than to whatever was reserved at boot.

The design falls out of that. Segregated free lists over slabs carved from
single frames, eight power-of-two classes from 16 to 2048 bytes; anything
larger takes a run of whole frames, which is contiguous in the direct map
precisely because it is contiguous physically. `libs/slab-layout` holds the
class arithmetic and is host-tested; `wost::mm::heap` holds the free lists,
which live *inside* the free slots and therefore need raw pointers.

Alignment is met by **choosing a class, never by padding**: a frame is
4096-aligned and a class's slots sit at multiples of the class size, so every
slot is aligned to its own class. A request needing 64-byte alignment takes
the 64-byte class whatever its size. That invariant is what the largest host
test asserts, over every slot of every class.

**Rationale.** Hand-rolled won, but not on the no-dependency principle — on
the boundary. None of the surveyed crates would have known to allocate through
the direct map, because none of them knows this kernel made mapping one-way.
The policy stays swappable: `slab_layout` decides where a request goes and
`heap.rs` is two hundred lines, so replacing first-fit-by-class with per-CPU
magazines is a change to one file (D-43's Safe Policy Injection, applied
again).

Slab frames are never returned. A slab whose slots are all free could be, but
knowing that needs a per-slab live count, and a kernel whose heap has stopped
growing has nothing to spend it on. `heap::stats` reports the numbers that
would justify adding one.

**Consequences.** `alloc` is enabled in the kernel; `Box` and `Vec` work. Three
on-machine checks, and the first of them is the lesson.

The obvious form of "a freed slot is reused" — allocate, note the address,
free, allocate again, require the same address — **is unsound**, and it failed
before passing for the wrong reason. Nothing obliges the compiler to keep two
allocator calls in source order when the values between them are dead, so the
second `Box` could be constructed before the first was released. Adding a
`debug!` between them made it pass. A check that changes verdict when a print
is added is measuring the optimiser, not the kernel.

The replacement counts instead of comparing: `live` moving and returning says
the round trip happened, `slab_frames` standing still says it was served from
the free list, and neither depends on where the slot is or on what the
compiler did with it. This is the second time in this phase that a check's
*early-exit or identity assumption* was the defect rather than the code under
test — the first was D-43's zeroing check — and both were found only because
D-28 requires breaking every new check once.

A lock-order note for Phase 4: `heap.rs` takes `FRAMES` while holding `HEAP`,
during a slab refill. Nothing yet allocates from the heap while holding the
frame lock, and nothing should — the inversion is the one deadlock this pair
can produce.

---

## D-48 The console scrolls out of RAM — Status: Accepted (2026-08-22)

**Context.** `001-boot.md` has carried a known limitation since Phase 1: the
framebuffer console reads uncached device memory back in order to scroll.
`007-memory.md` scheduled the fix as the heap's first consumer, and
`008-research-notes-2.md` §3.4 argued something stronger — that the back buffer
and the write-combining decision are *one* decision, because WC makes writes
fast and reads catastrophic, so the back buffer is the precondition that makes
WC adoptable rather than a complement to it.

**Decision.** The back buffer lands now, in slice 4, as `007` scheduled it.
Write-combining does **not** land with it, and the reason is a boundary the
research pass did not have: WC needs the kernel to map the framebuffer *itself*
with PAT bits, and D-45 keeps device holes out of the direct map precisely so
that devices are mapped deliberately. That mapping is slice 5's work. WC moves
to slice 5 with the argument intact — it is still gated on the back buffer, and
the back buffer now exists.

`Framebuffer` gains a shadow: one `u32` per pixel at the device's own stride.
Every write goes to both; every read comes from the shadow. `attach_back_buffer`
is separate from construction because the framebuffer is built during the boot
handoff, long before there is a heap — and it seeds the shadow by reading the
screen once, so attaching it mid-boot does not lose the lines already printed.
That single read is the last one the design permits.

**Rationale.** Splitting the two halves is not a retreat from §3.4's argument;
it is the argument applied in the right order. The dependency runs
back-buffer → WC, so doing the back buffer alone leaves nothing incorrect,
while doing WC alone would have made every scroll dramatically worse. The
research pass identified the coupling correctly and the sequencing loosely.

`try_reserve_exact` rather than `vec!`: a framebuffer shadow is megabytes, it
is the largest single allocation the kernel makes, and a console that cannot
get one should keep working rather than abort. Failure is reported and the old
read-back path stays.

**Consequences.** One on-machine check, and `001-boot.md`'s limitation is
retired. The helpers behind the check are `#[cfg(feature = "selftest")]` — the
three-pass gate D-22 gained in the audit caught them as dead code in the
configuration that ships, which is exactly what that pass exists for.

Writes now cost twice: once to the device, once to RAM. That is the trade, and
it is the right one — a scroll moves a screen's worth of pixels and previously
*read* every one of them uncached, while a glyph write touches a few hundred
words. WC in slice 5 will make the device half of every write cheaper still.

---

## D-49 Devices are mapped before the switch, and the keyboard interrupts — Status: Accepted (2026-08-22) (amended by D-76 and D-77)

**Context.** `007-memory.md`'s slice 5 owes three things: the I/O APIC up,
IRQ 1 routed, and D-37's polled keyboard drain retired. D-33 called the
unmappable device hole "the wall itself".

Designing it surfaced a consequence of D-45 that neither `007` nor
`008-research-notes-2.md` anticipated. `activate` **consumes** the address
space, so nothing can map into a table the CPU is already walking — that is
what makes `AddressSpace::map` a safe function. But a device's register window
is a mapping, and the device is discovered by reading ACPI, which happens
after boot. Taken naively the two requirements contradict.

**Options.** (a) Reopen mapping after activation behind a second guarded API;
(b) reserve a device window and map lazily on fault; (c) discover devices
*before* the tables are built, and map their windows during construction.

**Decision.** (c). ACPI is read while the bootloader's direct map is still the
live one, so the tables are already reachable; the I/O APIC's window is mapped
during `build`, and the claim on it is redeemed after the switch. The ordering
is forced by the design rather than chosen, and it is stronger for it: a
device the kernel has not decided to map is not merely unmapped, it is
unmappable.

`AddressSpace::map_device` puts device windows in a reserved range
(`0xffff_ff00_..`), uncacheable and never executable, well clear of the direct
map below and the image above. It returns a `DeviceMapping`, which is neither
`Copy` nor `Clone` — and that is what makes `DeviceMapping::claim` a safe
function returning an `MmioRegion`. Every clause of `MmioRegion::new`'s
contract is discharged by construction: the mapping is live (checked against
the active tables), permanent (nothing unmaps), and exclusive in both virtual
and physical interval registries (the token cannot be duplicated). RAM and
firmware-backed storage are ineligible for device mapping. This is the
sensitivity discipline `008` §4.1 took from
Asterinas, in the form this kernel can actually enforce: `port::claim` answers
"may two drivers have this?", and ownership of a `DeviceMapping` answers
"may anyone have it at all?"

**The parsing is host-tested, and that mattered.** `libs/acpi-tables` reads
the RSDP, the XSDT and the MADT from `&[u8]`, validating both RSDP checksums
and every table's, and refuses rather than panics on anything malformed.
Firmware is the least trustworthy input the kernel has: the host suite covers
truncated tables, corrupt checksums, an entry claiming zero length (which would
otherwise loop forever), an entry running past the table, and a truncated entry
of a known kind. `wost::mm::FirmwareMemory` copies only from ACPI storage, and
the parser refuses a corrupt tail before exposing a valid prefix.

**Routing does not assume `irq == gsi`.** The MADT's interrupt source
overrides are read and honoured, with the MPS INTI polarity and trigger
encodings decoded rather than defaulted. On QEMU the keyboard is GSI 1 either
way; on hardware that remaps it, assuming identity is the classic way to
program an I/O APIC that then delivers nothing.

**Consequences.** D-37 is retired: the idle loop no longer polls, and
`route_keyboard` declines gracefully to the old path if firmware described no
controller, if the GSI is not this controller's, or if the controller will not
raise its line.

Two on-machine checks, and the *third* one is where this got interesting. The
existing "an injected keystroke arrives" check kept passing throughout the work
— because `await_key` polled the controller itself, so it proved a keystroke
arrived and said nothing about how. Rewriting it to watch what the *handler*
delivered turned it red immediately: the line was routed, the vector was
right, and nothing was being delivered, because Phase 2 configured the i8042 to
raise no interrupts at all (D-37, and correctly so at the time). One
`enable_interrupt` later it went green.

That is the third check in this phase whose *shape* hid a defect, and the
pattern is now unmistakable: D-43's zeroing check short-circuited on an
identity comparison, D-47's reuse check depended on allocator call ordering,
and this one measured the wrong path entirely. All three looked reasonable. All
three were found by D-28's break-it-once rule or by tightening a check that had
never failed. A check that has never failed is not evidence; it is an untested
claim.

Phase 2's "the controller raises no interrupts and does not translate" is
replaced by one that ties the configuration to the routing: the first port
raises its line **exactly when** something routed it, translation is off in
both worlds, and the second port is never enabled.

**Left for a future phase, recorded so it is not rediscovered:** a device
discovered *after* boot — anything hot-plugged, and every PCI device in Phase
8 — cannot be mapped under this design. The answer is not to weaken `activate`
but to give the kernel address space a way to grow that keeps the same
property, which is a decision Phase 5's address-space work should take
deliberately rather than by accident.

---

## D-50 The framebuffer is write-combining — Status: Accepted (2026-08-22) (amended by D-77)

**Context.** `008-research-notes-2.md` §3.4 argued that write-combining and the
console back buffer are one decision, with numbers: Linux's own MTRR
documentation puts WC at "2.5 times or more" for image writes, and field
reports on framebuffer redraw run far higher. D-48 accepted the coupling and
corrected the sequencing — the dependency runs back-buffer → WC, so the back
buffer landed in slice 4 and this waited for the device-mapping machinery
D-49 built.

**Decision.** The page attribute table's second entry is changed from
write-through, which this kernel never asks for, to write-combining; every
other entry is preserved from the value the machine supplied. The
framebuffer's only direct-map alias selects that entry. The transition writes
PAT before activating the new tables, reloads `CR3`, and restores the prior
PAT byte and reloads again if activation fails.

**Rationale.** WC coalesces writes in a fill buffer instead of putting each on
the bus, and makes reads dramatically worse — which is exactly the trade a
framebuffer wants and exactly why it could not be taken before slice 4. After
the back buffer, the console reads the device precisely once, when the shadow
is seeded.

The shadow already holds every pixel, so after attachment nothing reads the WC
mapping. Keeping one physical alias also avoids the undefined cache-type
conflict the original two-alias design admitted.

**Consequences.** One on-machine check, and it has two halves for a reason. The
PAT entry must actually be write-combining — a hypervisor may decline the MSR
write, which is why `write_combining_is_enabled` reads it back rather than
remembering what was written — *and* the mapping must select it. Either alone
leaves writes crossing the bus one at a time while the log claims otherwise.

The old claim that a wrong memory type could only be slow was false: conflicting
cache types for aliases can violate the processor's rules. Safety now rests on
one alias, a checked framebuffer extent and format, a read-modify PAT update,
and an activation guard that either commits or restores the transition.

Phase 3's slice 6 closes with this, and with it the phase: every debt
`007-memory.md` collected is paid except D-46's KASLR, which is deferred to
Phase 12 with its cost named.

---

## D-51 The context switch is a naked function, and a switch is a safe call — Status: Accepted (2026-08-22) (amended by D-59 and D-76)

**Context.** `008-research-notes-2.md` §1 verified `#[unsafe(naked)]` and
`naked_asm!` against the pinned toolchain and named Phase 4's context switch
as the natural place to adopt them. The only other assembly in the kernel is
the interrupt trampoline, a `global_asm!` blob that sits outside every
mechanism `005` mandates — no signature, no `///`, no `# Safety`.

**Options.** Another `global_asm!` blob (a symbol the type system cannot see);
a naked function (a Rust item with a signature and a contract); convert the
interrupt trampoline to match (churn on the most battle-tested assembly in the
repo, for no behavioural change).

**Decision.** `wost::task::switch` is a naked function: push the six
callee-saved registers, store `rsp`, load the other, pop, `ret`. Nothing
else crosses — the ABI already lets a call clobber the rest, and the kernel
is built without SSE so there is no floating-point state to carry. The
interrupt trampoline is **not** converted: that is a judgement about churn,
taken, and it may be revisited when the trampoline next changes for a reason
of its own.

The switch is `unsafe` and private. What the kernel calls — `switch_to`,
`exit_to`, `fabricate` — is safe, on the same grounds that make `activate`
safe (D-45): every value the switch loads can only have come from this
module. A saved pointer is written by a previous switch or by `fabricate`;
`fabricate` writes only on a stack the arena mapped and `init` walked in the
live tables; a slot that has exited is marked dead and refused by every
later switch, so nothing can resume onto a stack that has since been handed
to a new thread. The kernel never holds a context value at all — it names
slots by index, and the table decides what each index may do.

**Rationale.** The sound-wrapper principle `005` holds every `wost` export to
asks not whether the assembly is right but whether any caller can reach
undefined behaviour. A public `Context` value would make that unanswerable —
two of them for one stack is a switch into a frame something else is standing
on. Keeping the state private and exposing only checked transitions answers
it. The naked function is the right shape for the assembly for the reasons
§1 gave: it is an item, so the lints reach it.

**Consequences.**

- The interrupt flag is clear across every switch and restored by a context
  that resumes. The founding implementation gave a new thread the switcher's
  flag; D-59 supersedes that rule and captures the creator's flag in
  `fabricate` instead.
- The founding fabricated frame was six zero words plus `thread_start`'s
  address. D-76 found that seven-word frame misaligned every new thread and
  added an eighth padding word; the current ABI argument is in D-76.
- The first break-once attempt — deleting the `r12` push and pop — did not
  test what it meant to: it changed the frame size, so the fabricated frame's
  `ret` popped a zero and the machine faulted. Saving `rax` in `r12`'s place
  is the break that tests preservation, and it turned the register check red.
  A break that crashes proves less than one that fails.
- Kernel-side, `THREADS` is a leaf lock and every switch happens after its
  guard drops; `wost`'s own table lock is taken inside `switch_to`, never
  under the kernel's. The lock order is in `009-multitasking.md`.
- Not yet enforced: that a thread does not yield while holding a lock. It
  cannot corrupt memory — the next thread simply spins with the flag it
  inherited — but it can hang, and D-55 (the context token) is where the
  type system takes it over.

---

## D-52 Thread stacks come from a boot-time arena, because mapping is one-way — Status: Accepted (2026-08-22)

**Context.** D-45 made `activate` consume the address space, so nothing can
map after the CR3 switch; D-49 deferred how the address space grows to
Phase 5, deliberately. Phase 4 needs stacks with guard pages for threads
that do not exist at boot.

**Options.** Stacks in the direct map (no hole can be punched for a guard
without unmapping, which D-45 forbids, and D-12 requires the guard); a
post-activation mapping API (weakens D-45 and pre-empts the decision D-49
reserved); a fixed arena mapped while the tables are being built.

**Amended by D-113**, which raised the stack to 32 KiB: the arena is sixteen
slots of a 32 KiB stack and costs 512 KiB, not the 256 KiB this entry works
through below. The reasoning is unchanged and the numbers here are left as they
were decided — the pointer is what D-45 and D-51 carry beside it, and this one
did not (D-146).

**Decision.** An arena: sixteen slots of a 16 KiB stack above a 4 KiB gap, at
`0xffff_fe00_0000_0000` — clear of the direct map, the device window and the
image. Every stack is committed, zeroed and mapped writable-and-NX in
`build`; every gap is simply never mapped. The arithmetic is
`libs/stack-layout`, host-tested for the property the design rests on: no
two slots touch, and every stack has its guard immediately below. `wost`
keeps the allocation and the map calls, and hands back a `StackArena` that
is neither `Copy` nor `Clone` — the evidence, on D-49's `DeviceMapping`
pattern, that `task::init` may fabricate on exactly these ranges.

The boot thread keeps the bootloader's stack. Moving it would mean a one-way
jump onto a new stack mid-`kernel_main`, and the only thing that buys today
is a guard under a stack that has never needed one; slice 4's idle thread is
the natural moment to retire it, if it is retired at all.

**Rationale.** Sixteen stacks cost 256 KiB of a machine with hundreds of
megabytes, and a cap that is a constant in one file is cheaper than a growth
mechanism designed before its constraints are known. An absent mapping is the
strongest guard there is and the cheapest.

**Consequences.**

- A real cap on threads until Phase 5's growth design, recorded as such.
- One more layout assumption for D-46's ledger: the arena's address is fixed
  and KASLR would have to move it.
- `008` §4.1's caveat is named rather than hidden: a frame larger than 4 KiB
  can jump the gap. The kernel has no `alloca` and no SSE spills, so the
  frames it builds are small; if that changes, the gap grows.
- `task::init` walks every stack in the *live* tables before accepting the
  arena — the arena says they were mapped, the walk says the CPU agrees.
- The check is structural: the guard is absent at both ends and the stack is
  present, writable and NX at both ends, for all sixteen slots. Mapping the
  guard turned it red.

---

## D-53 Scheduling policy is arithmetic in `libs/eevdf`, behind a seam with a fallback — Status: Accepted (2026-08-22)

**Context.** D-15 chose EEVDF and wanted pluggable policy as "a cheap future
door, not a v1 feature"; `008-research-notes-2.md` §4.4 took sched_ext's
lesson as a constraint on the interface — policy *separable and observable,
with a fallback path* — and flagged the lag rule as the part that is easy to
omit.

**Decision.** The policy is `libs/eevdf`, pure functions over plain data:
virtual time that wraps and compares by signed difference, charging scaled
by weight, a weighted average as the fair-share point, eligibility as
non-negative lag, the pick as earliest deadline among the eligible, and
placement that clamps lag to one slice either way. Thirteen host tests
including a simulation that drives the policy the way the kernel does and
asserts a 1:2 weight ratio lands at 1.9–2.1, and a sleeper test that asserts
the clamp is what stops a returning thread running unopposed.

The kernel's `thread.rs` is the mechanism side of the seam: it keeps an
`Entity` per thread, feeds the runnable ones to the policy, and acts on the
answer. Candidates are considered in rotation from just after the running
thread, so equal deadlines take turns rather than the lowest index winning
every tie. If the policy ever answers "nobody" when everyone was considered —
which its own tests say cannot happen — the kernel takes the next runnable
slot round-robin and counts it, and a self-check requires the count to be
zero. That is the fallback in its smallest honest form: not a second policy,
but a way to keep the machine up and make the inconsistency visible.

**Rationale.** Arithmetic that decides who runs is exactly what D-30's
pattern exists for; on the host it can be driven a thousand steps in a
millisecond. Separability costs nothing — the seam is a function signature —
and observability is one counter.

**Consequences.**

- Virtual time carries a `RESOLUTION` of 1024 units per unit of real time,
  because the fairness simulation found the heavy thread's clock standing
  still: one millisecond at twice the base weight truncated to nothing. The
  test that found it is the test that guards it.
- Real time is charged in milliseconds from `uptime_ms`, and the start
  stamp always advances to `now`, so time is conserved across stints even
  when single stints are shorter than a millisecond — it is attributed
  coarsely, never lost. Slice 3's finer clock refines the attribution.
- The yield path excludes the running thread from the candidates but not
  from the average, so "nobody else is eligible" is a real answer there and
  the thread keeps the CPU; the fallback counter is reserved for the
  preemption path, where everyone is considered. The first boot counted a
  fallback for exactly this reason, and the counter earned its keep on day
  one by saying so.
- Lag is clamped, not decayed: preserved across a sleep so a brief sleep
  sheds no debt, capped at one slice so a long one banks no credit. That is
  mainline's rule; the decay `008` described is a proposal there, and it can
  be revisited when something here can sleep (slice 4).

---

## D-54 Preemption: cooperative first, then the timer at trap exit, with "was interruptible" as the test — Status: Accepted (2026-08-22) (amended by D-76)

**Context.** Slice 1 proved the switch under yields alone. Something had to
take the CPU from a thread that never gives it up, and the question was
where that switch may happen and what must stop it.

**Options.** Preemptive from the first commit (couples the two hardest new
mechanisms in one change); a deferred-reschedule flag acted on at some later
safe point (another state to keep consistent); a switch from inside the
timer's handler, gated on whether the interrupted code could be interrupted.

**Decision.** The tick handler dismisses the interrupt and then calls
`thread::reschedule` with the trap frame. It switches only if the frame's
interrupt flag was set. That test is free and exact: D-34 clears the flag
while any lock is held, and the scheduler itself clears it from decision to
switch, so every critical section in the kernel is unpreemptible without a
preempt count, and nothing is ever switched out of the middle of a decision.
`wost::task::switch_from_trap` makes the frame the evidence — it refuses a
frame whose flag is clear, and refuses to be called outside a handler.

The handler's own frame stays on the preempted thread's stack; what the
switch saves is the handler's path down to it. When the thread is next picked
it resumes inside its own handler, returns through the trampoline, and
`iretq` restores the flags the CPU pushed.

Asterinas's `PreemptDisabled` guard discipline, which D-34 left for "once
there is a scheduler", is **declined**: with the flag as the test, a lock
*is* a preemption guard, and a second lock type would be a type parameter
with one useful value still.

**Rationale.** The ordering — end-of-interrupt before the switch — is not a
detail: the APIC holds the timer's priority class until the EOI, and a thread
running inside it would hold every later tick hostage. Cooperative first was
the right sequencing: every one of slice 1's checks ran under slice 2's
preemption unchanged, which is what proving the switch in isolation bought.

**Consequences.**

- A thread that was preempted inside a handler and one that yielded resume
  by different paths, and both work without either knowing which it is; the
  switch does not care what is above it on the stack.
- Two checks: a spinner cannot starve the boot thread, which also spins
  (red with preemption disabled), and runtime divides 1:2 by weight within
  coarse bounds (red with the weight ignored).
- One hundred ticks a second now each cost a lock and a pick. Slice 3
  retires the tick and the cost with it.

---

## D-55 Trap context is a count the heap and the scheduler can ask — Status: Accepted (2026-08-22)

**Context.** `008-research-notes-2.md` §4.2 named the bug class no lint
here can catch — sleep-in-atomic-context, and its memory-safety cousin,
allocation from a handler that interrupted the allocator's own refill — and
said the idea was expressible on stable as a context token in the
`PanicToken` idiom, worth an ADR when the heap landed. The heap landed in
D-47; slice 2 adds the first switch out of a handler, which is where the
rule starts being breakable.

**Decision.** `wost::interrupt` keeps a trap-depth count: incremented around
every handler call, zero in thread context. The heap's `alloc` asserts it is
zero — loudly, because a silent null would be blamed on memory. `yield_now`
asserts it is zero. A switch out of a handler parks the count with the
thread it belongs to, so the thread switched to sees its own depth and the
preempted one gets its back on resumption.

The token *type* — a value only thread context can mint, taken by every API
that may sleep — is deferred to the first API that may sleep, which is
`WaitQueue::wait` in slice 4. Today's two callers assert against the count
directly, and a token over two call sites would be ceremony. Klint itself is
a custom driver and out under D-01.

**Rationale.** The count is the mechanism either way; the token is sugar
over it that pays off when there are enough call sites to forget one. The
allocation assert is the half that matters now, because it is the half with
memory safety behind it.

**Consequences.**

- Nothing in the kernel allocates in trap context today, which the first
  boot with the assert confirmed by not panicking — a fact that was not on
  record before.
- Not yet enforced: yielding while holding a lock. It cannot corrupt memory
  (the next thread spins with the flag it inherited), only hang. The token
  of slice 4 is where it becomes a type error.

---

## D-56 Per-CPU state is one word, shaped to widen — Status: Accepted (2026-08-22)

**Context.** One CPU today; SMP is a later phase. The scheduler needs "which
thread is on this CPU", and how that is stored decides whether SMP is a
widening or a rewrite.

**Decision.** `wost::task::CURRENT` is a single atomic word, read and written
only under the task table's lock with the flag clear. Everything that is
per-CPU in principle — the current slot, the trap depth, the interrupt
hand-off flag — is a `static` today and is named as such in its doc comment,
so the SMP phase has a list rather than a hunt. GS base and `swapgs` stay
where D-29 left them, waiting for ring 3.

**Rationale.** A per-CPU abstraction designed before there is a second CPU
would guess at what it needs (APIC IDs, stack switching on entry, IPIs) and
be wrong in the details. Keeping the statics few and named is the cheap form
of the same intent.

**Consequences.** The SMP phase owes: `CURRENT` and `TRAP_DEPTH` become
per-CPU cells, `prepare`'s single-CPU exclusivity argument becomes a per-CPU
one, and D-45's TLB-shootdown debt comes due the moment anything unmaps.

---

## D-57 The tick is retired: the counter is the clock, the timer is an alarm — Status: Accepted (2026-08-23)

**Context.** D-15 wanted tickless from the start and D-36 built a 100 Hz
periodic tick anyway, calling it "explicitly disposable when D-15 lands" —
because a tickless kernel decides when next to wake, and that decision
belongs to a scheduler. Slice 2 built the scheduler. D-36's tick also owed
two debts: a critical section longer than a tick swallows one, and uptime
interpolates the countdown the tick reloads, so retiring the tick deletes
uptime's denominator along with it.

**Options for the timebase.**

- *Accumulated deadlines.* Sum each one-shot interval as it expires, plus
  where the countdown now stands. Needs no new hardware, and has a hole
  exactly where tickless puts one: while nothing is armed, nothing accrues,
  so an idle machine's clock stops. Closing it means arming a housekeeping
  timer, which is the tick under another name.
- *The HPET or the ACPI power-management timer.* Real free-running counters,
  both reached through tables and mappings this kernel does not yet parse,
  and both an order of magnitude slower to read than a register.
- *TSC-deadline mode.* Arms the timer from a comparison against the counter
  rather than from a countdown. A different interface, not a different
  timebase, and it needs everything below first.
- *The timestamp counter.* Free-running, one instruction to read, and its
  rate is measurable by exactly the machinery D-36 already built — the same
  twenty-millisecond window, with two more reads inside it.

**Decision.** Two clocks, measured in one window, each doing the job it is
shaped for. The timestamp counter is the clock: `now_ns` is a subtraction and
a multiply. The APIC timer is the alarm: one-shot, armed for the next moment
the scheduler has a reason to look — the running thread's slice ending, or
the earliest sleeper's deadline — and **stopped** when there is neither.
Every path that can change that answer ends in the same call, so the arming
rule lives in one place.

The conversions are `libs/timebase`: a rounded multiply-and-shift, thirteen
host tests. The multiplier is held to thirty-two bits, which buys two things
— any `u64` times it fits a `u128`, and a whole conversion packs into one
word, which is how the clock is published to a reader that cannot take a lock
because it is inside an interrupt handler.

**Rationale.** A counter that free-runs is the only thing that makes an idle
machine's clock keep time, and an idle machine's clock keeping time is the
whole difference between tickless and a tick with extra steps. Reading it is
a multiply because the read path is every scheduling decision; a sixty-four
bit division there would be paid for thousands of times a second.

**Consequences.**

- **The monotonic clamp is deleted, and that makes its check mean
  something.** D-36 clamped every reading against a high-water mark because a
  reader with the flag clear could watch the countdown reload while the tick
  it announced stayed pending. There is no countdown to reload now, so
  monotonicity comes from the counter, and "uptime never runs backwards" has
  stopped being a check that could not fail. Breaking it — one exclusive-or
  on the low byte of every reading — turned it red for the first time.
- **The invariant-counter bit is reported, not obeyed.** `cpuid` leaf
  0x8000_0007 is the architectural promise that the counter runs at a
  constant rate, and a machine without it is warned about by name at boot. It
  is not a veto: QEMU under TCG refuses to advertise the bit — the gate's own
  machine — while providing a perfectly steady counter, and the evidence this
  kernel actually has is two independent windows that agreed to one part in
  sixty-four. Refusing to boot on the promise while holding the evidence
  would be the wrong way round. VMware advertises it, which is where the
  phase's D-18 boot observes it true.
- **Time is nanoseconds now, and the slice is three milliseconds** — a tenth
  of what D-53 could afford, because the cost of a short slice used to be a
  tick that could not resolve one and is now a timer armed for exactly it.
  D-53's coarse-attribution debt is paid: stints are charged in nanoseconds.
- Two clamps bound what may be asked of the alarm. Below, fifty microseconds:
  a deadline already passed would otherwise arm one tick of a gigahertz clock
  and take the interrupt again immediately. Above, the counter's thirty-two
  bits — about four seconds at a gigahertz — after which the expiry re-arms
  for the remainder rather than being handed a wrapped count.
- The debt D-36 recorded is discharged rather than reduced. A deadline that
  passes while the flag is clear is delivered late, not lost, and uptime does
  not undercount because it never depended on the interrupt.
- Deleted with the tick: `x86_apic::timer::elapsed_milliseconds` and its
  interpolation tests, `x86_pit::calibrate::periodic_initial_count`,
  `apic::start_periodic_timer`, `time::ticks`, `MS_PER_TICK`, and the two
  self-checks that counted ticks. `calibrate::divided_hz` is now
  `measured_hz`, because one window measures two clocks.
- Six checks on the machine, each broken once. The one that matters is **an
  idle kernel takes no timer interrupts** — not fewer, none — against the
  fifty the retired tick would have taken in the same half second. Its
  counterpart under load bounds the same counter from both sides, and is also
  what proves the handler's end-of-interrupt, which the retired tick's pair of
  checks used to.
- Two of the breaks hit the wrong path first, which is now three times this
  phase. Making the measured counter rate a thousand times too fast turned the
  machine into an interrupt storm — a three-microsecond slice — before the
  check that reads the rate ever ran. And breaking the expiry handler's
  re-arming could not redden a check about an *idle* kernel, which by
  definition never enters it; the break that reaches it is one to the call
  that stops the timer.

---

## D-58 Sleeping: a deadline, a preserved lag, and a halt for the degenerate case — Status: Accepted (2026-08-23)

**Context.** Tickless is only worth building if something wants to wait. D-53
built lag-clamped placement — a sleeper keeps its debt and banks at most one
slice of credit — and noted that nothing could sleep yet, so the clamp had no
caller. This is the caller.

**Decision.** A thread state, a deadline, and a saved lag. `sleep_ms` charges
the thread, records the lag it is leaving with, marks it sleeping until an
absolute deadline, and hands the CPU to whoever the policy picks. Every
scheduling path wakes whatever is due before it decides anything, and a woken
thread is placed back at the queue's fair-share point offset by the lag it
left with — which is where D-53's clamp acts, at last against a real sleeper.

The wait is a loop, not a single switch: a thread can be woken by something
other than its own deadline, and the deadline is what was promised.

**The degenerate case is deliberate.** A thread that sleeps with nothing else
runnable keeps the CPU and halts on it — still the running thread, still the
one the table charges, waiting on the timer it armed for itself. Enabling
interrupts and halting has to be one instruction pair, or the interrupt that
was going to end the wait lands in the gap and the CPU halts with nothing
left to wake it; `interrupt::enable_and_halt` is that pair, and the
architecture's interrupt shadow is why it works.

**Rationale.** Slice 4's idle thread removes the degenerate case by making
something always runnable, and inventing that thread here to avoid a six-line
branch would have pre-empted a decision this phase's charter gives to slice 4.

**Consequences.**

- A sleep on a machine whose clock never started returns at once. There is no
  deadline to wait for and no timer to be woken by, so a wait there would be a
  hang — and a kernel that hangs tells the harness nothing, which is worse
  than one that tells it something wrong.
- The polled keyboard path D-37 left behind was driven by whatever the tick
  woke. It now sleeps ten milliseconds at a time between drains, which is the
  same rate restated as the interval it always was, asked for by the only code
  that needs it.
- Two checks, one per branch, each broken once. A sleeper with nothing else to
  run comes back on time and the machine takes **one** timer interrupt while
  it waits, bounded on both sides so a conversion wrong in either direction
  shows. A sleeper with a competitor hands the CPU over, counted by a wake
  counter that only the sleeping-to-ready path moves.
- Still owed: nothing can be woken *early*, by an event rather than a
  deadline. That is `WaitQueue`, and it is slice 4's.

---

## D-59 A thread's first interrupt flag comes from its creator — Status: Accepted (2026-08-23)

**Context.** A thread resuming after a switch restores its own flag from its
own stack. A thread running for the first time has no such record, and D-51's
`wost::task` gave it the switcher's, "the same rule a lock guard follows: put
back what was found". The switcher is a scheduler, and a scheduler clears the
flag around every decision it makes.

**What that cost.** Every thread whose first run came from `switch_to` — the
yield path, and now the sleep path — started with interrupts disabled and
could never get them back. Nothing noticed from slice 1 until slice 3, because
the checks that used the yield path had their threads yielding constantly, and
the check that needed preemption got its thread started by the preemption path
instead, where the flag handed over is the interrupted code's and is set. The
symptom, when it came, was a hang: a thread slept, handed the CPU to a spinner
that could not be interrupted, and the machine stopped.

**Decision.** The flag is captured in `fabricate`, from the thread's creator,
and restored by the entry trampoline. It cannot be captured at the switch: by
then the only flag left to read is the scheduler's own.

**Rationale.** A new thread holds no lock and has nothing to protect, so there
is no state it inherits that would justify running uninterruptible. The one
thing worth inheriting is whether the machine was accepting interrupts at all
when the thread was made — which is what its creator's flag says, and which is
correctly `false` on a machine where interrupts never came on, where
unconditionally enabling them would panic.

**Consequences.**

- `HANDOFF_INTERRUPTS` is gone, and with it the only entry on D-56's per-CPU
  list that was not really per-CPU: this belongs to a slot, not a processor.
- A thread spawned from inside a critical section still starts
  uninterruptible. That is the inheritance rule applied honestly rather than
  an oversight, and it is deterministic and documented; nothing in the kernel
  spawns from one.
- One check, broken once: a spawned thread records the flag on its first
  instruction, and it must be set. Spawned from a yield, because that is the
  path that got it wrong — the preemption path would have passed either way.
  Breaking it reproduced the original hang exactly.

---

## D-60 The idle thread is outside the policy, not at the bottom of it — Status: Accepted (2026-08-23)

**Context.** Slice 3's sleep had a branch for "nobody else is runnable": the
sleeping thread kept the CPU and halted on its own stack. Correct, but it
exists only because there is sometimes nothing to run, and everything after
this phase is easier if something always is.

**Options.** A thread of the lowest weight the policy will take; a second
scheduling class; or no idle thread at all, keeping the halt-in-place branch
everywhere it is needed.

**Decision.** A thread of its own with `Class::Idle`, which the policy never
sees. It is not in the fair-share average, it is not counted when deciding
whether the timer needs arming, and it is picked only when no normal thread
is runnable. `start_idle` is idempotent and runs after interrupts are on,
because a thread that halts needs them to be woken from.

**Rationale.** A weight small enough to ignore is still a weight: EEVDF would
hand an idle-weighted thread the CPU every so often on eligibility grounds,
and every one of those is a wakeup spent on nothing. The vision doc counts
wakeups. Keeping the class out of the policy is four lines and makes the
tickless claim exact rather than approximate — an idle machine still arms
nothing.

**Consequences.**

- The halt-in-place branch stays, because the idle thread does not exist
  during early boot or on a machine where the spawn failed. The self-checks
  start the idle thread half way through their own run, so both sides of it
  are exercised: the sleep checks above it have no idle thread, the checks
  below it do.
- Two checks, both broken once. The idle thread runs when the boot thread
  sleeps, and does not run at all while a spinner is runnable. Either alone is
  satisfied by a bug — an idle thread that never runs, or one the policy
  schedules like any other — and the second break, making it a normal thread,
  reddened four checks including the tickless ones.
- SMP owes it one idle thread per CPU, which is one more entry on D-56's list.

---

## D-61 Waiting on an event: a bitmask queue, and a reschedule at trap exit — Status: Accepted (2026-08-23)

**Context.** D-58 gave the kernel a wait for a *time*. The keyboard needs a
wait for an *event*: D-49 routed the line and retired polling, and the debt it
left was that nothing could block on a key.

**Decision.** `WaitQueue` is a bitmask of thread slots in the queue itself.
`wait` registers, blocks, and hands the CPU on; `wake_all` takes the mask and
makes those slots ready. No allocation anywhere on the path, because the waker
is an interrupt handler and D-55 forbids it the heap. A woken thread is placed
back at the fair-share point offset by the lag it parked with — the same rule
sleepers get.

The keyboard queues decoded events in a fixed ring and wakes the readers. The
kernel's own last act is to become one: the boot thread blocks on `read_key`
and draws what arrives.

**A wake needs a reschedule, and trap exit is where one is legal.** A thread
made runnable by a keystroke has no appointment to be woken at — there may be
no next expiry at all — so the keyboard handler asks the scheduler after its
end-of-interrupt, exactly as the timer does, and only when something woke.

**Rationale.** The lost-wakeup race is the classic bug here, and the fix is
that checking the queue and registering as a waiter happen together under a
clear interrupt flag. A queue's lock and `THREADS` are never held at once, so
the lock order gains a second leaf rather than an edge.

**Consequences.**

- A kernel at a prompt takes exactly one interrupt per keystroke. Nothing is
  armed for a thread with no deadline, so the machine is genuinely idle
  between them — which is the whole of Phase 4 in one sentence.
- Two checks. A blocked reader is off the run queue *and* the machine is quiet
  while it waits; then it wakes with the key the harness pressed. D-49's lesson
  shaped both: the first version of the second check claimed the reader had
  not polled and could not tell, and a deliberately polling `read_key` passed
  it. It now records whether it blocked, across its own call.
- The dropped-event counter should stay at zero and is checked. Sixteen
  events is generous against a reader that blocks; a burst larger than that
  means nobody is reading.
- Still owed: nothing can wait on a queue with a timeout, and nothing can wait
  on two. Both wait for a reason to exist.

---

## D-62 The context token arrives with the API that needed it — Status: Accepted (2026-08-23)

**Context.** D-55 built the trap-depth count and deferred the *token* —
`008-research-notes-2.md` §4.2's idea — until there was an API that could
sleep, on the grounds that a token over two call sites would be ceremony.
D-58 and D-61 are that API.

**Decision.** `ThreadContext` is a zero-sized token minted only by
`ThreadContext::claim`, which refuses in trap context. `yield_now`,
`sleep_ms` and `WaitQueue::wait` take one by reference. It is neither `Send`
nor `Sync`, so it cannot be stashed in a `static` and produced later by
something with no right to it.

**Rationale.** The count was always the mechanism; the token is what turns
three run-time asserts into one compile-time rule. With three call sites and
more coming in Phase 5, it now pays for itself.

**Consequences.**

- `ThreadContext::require` exists for the two places that are structurally in
  thread context and would rather panic than thread an `Option` through —
  the boot thread's final loop, and the self-checks.
- The token proves the *context*, not the absence of a lock. Yielding while
  holding one is still possible and still only a hang. A guard-carrying
  variant is the obvious next step and has no caller yet.

---

## D-63 The console keeps interrupts on, and a lock kind is a rule — Status: Accepted (2026-08-23)

**Context.** D-34 accepted that interrupt latency is bounded by the longest
critical section, named no bound, and declined a second lock type as "a type
parameter with one useful value still". This phase's charter required a
position: fix, or accept with the bound named.

**What the measurement said.** A console scroll on the gate's own machine is
seventy milliseconds — a framebuffer moved and blitted under one lock, with
interrupts off for all of it. That is not a latency for any interrupt to wait
behind, and naming it would have been accepting it.

**Decision.** `wost::sync::ThreadLock`: a spinning lock that does *not*
exclude interrupts and refuses to be taken from trap context. Both console
devices use it. The rule that makes it sound — nothing logs or draws from a
handler — was already D-31's, stated in prose; it is now an assert against
D-55's trap depth. The keyboard handler's echo, and its `debug!`, moved to the
reading thread.

*Corrected 2026-08-23 by the phase 0–4 audit.* This said the handler's
`debug!` drew from a handler. It never did: `MAX_LEVEL` is `Info`, so `record`
returns before touching a device. The echo was the live violation; the log
line was a latent one that would have become real the moment the level was
raised — which is the better argument for moving it, and the one the assert
now makes unnecessary to remember.

**And a second rule, found by breaking the first.** Putting the console back
on a `SpinLock` to check that the new check went red made the machine hang
instead: the log path then took an interrupts-on lock from inside an
interrupts-off one, and the preempted holder could never be scheduled. So
`SpinLock` counts its holders and `ThreadLock` refuses to be taken while any
is held. Two locks each individually correct, deadlocking together, is
exactly the kind of thing a comment does not catch.

**Rationale.** D-34's reasoning was right for its time and its conclusion has
expired: there are two useful values now, and the second one is what lets the
longest operation in the kernel stop being everyone's latency floor.

**Consequences.**

- A `ThreadLock` holder can be preempted, which a `SpinLock` holder cannot.
  That costs a waiter the rest of its slice and no more, because the timer
  takes the CPU back and gives it to the holder — the scheduler is what makes
  the trade safe, which is precisely what D-34 said it was waiting for.
- The panic path is exempt from both asserts: the code it interrupted never
  resumes.
- The check is not how long a scroll takes but that interrupts keep arriving
  while it runs, counted from inside the lock. Sampled outside it, a pending
  expiry lands the instant the guard drops and the count moves either way —
  which the first version did, and it passed under the break.
- The keystroke watch had to change with it. It kept the last press and the
  last release, which works only if a reader looks between one keystroke and
  the next; once the handler stopped logging it became fast enough to decode
  a whole injected burst in one drain, and a check waiting for the first key
  of that burst waited out its ten seconds. Two accumulating bitmasks now.
- Still owed: `SpinLock` holders are counted globally, which is one more
  entry on D-56's per-CPU list.

---

## D-64 A physical read is bounded by the memory map, not by a comment — Status: Accepted (2026-08-23) (amended by D-76 and D-77)

**Context.** The phase 0–4 audit found two `// SAFETY:` comments asserting
checks that no code performed. `mm::read_physical` said `direct_pointer` had
confirmed the direct map covers the range; `direct_pointer` only checks that
the arithmetic does not overflow. `DeviceMapping::claim` said its region was
four-byte aligned; nothing established that, and the window deliberately keeps
the device's own offset from a page boundary.

Neither was reachable. `read_physical` runs before `activate`, where the
bootloader's map covers everything; every device this kernel maps happens to
be page-aligned. Both were one firmware quirk from being real, and
`read_physical`'s length comes from a firmware table's own header.

**Decision.** There is no general safe physical-read operation. A
`FirmwareMemory` capability copies with volatile reads only from ACPI
reclaimable or ACPI NVS regions, and refuses overflow, gaps, every other memory
kind, and buffers outside a one-megabyte table budget. ACPI parsers also enforce
their declared length, containing buffer, checksum, fixed header and complete
entry stream before exposing any entry. A device whose registers are not
four-byte aligned is refused at map time, with an error that says so.

**Rationale.** The previous audit's standing lesson was that an abstraction
promising a bound must be given something that can check it. A safety comment
is not that thing, and a memory-map description is not authority to read live
kernel data. The capability names both the permitted provenance and the copy
semantics; the parser supplies the second, hostile-input bound.

**Consequences.**

- The machine check proves both acceptance of the real firmware bytes and
  refusal of non-ACPI storage. Host checks separately break every ACPI length,
  checksum and entry-stream bound.
- `FirmwareMemory::contains` walks adjacent regions but permits only the two
  ACPI kinds throughout the span. No safe caller can borrow an alias to the
  underlying physical bytes.
- The `Frame` methods kept their claim but got the true argument for it: a
  frame is allocatable memory, and every allocatable region is direct-mapped.

---

## D-65 The gate boots three kernels — Status: Accepted (2026-08-23) (a fourth added by D-90)

**Context.** The 2026-08-22 audit's other standing lesson was that a gate is
only as wide as its configuration matrix. This one found the matrix two
configurations short. `ci` booted the self-test kernel and *built* the shipped
one; and it was release-only, where arithmetic wraps in silence.

The first gap had grown teeth. The self-test build exits at the debug-exit
device the moment it has a verdict, so nothing it does covers what the shipped
kernel does afterwards — and since D-61 made the boot thread a blocking
keyboard reader, that is everything the kernel does at all.

**Decision.** Three boots. The self-test kernel in release, the same in debug,
and the shipped image under `boot-check`, which requires it to reach its
steady state and then survive five more seconds without panicking. Fifteen
seconds of extra wall clock between them.

**Rationale.** Both gaps were found by reading rather than by failing, which
is the argument for closing them: a hole in the gate is invisible from inside
it. The debug boot is the only configuration in which an overflow on the
machine goes red rather than quietly wrong, and it caught one on its first
run — in the round counter written that hour, where `then_some` evaluates its
argument even at zero.

**Consequences.**

- A debug boot gets 150 seconds where release gets 60. A debug kernel is
  several times slower under emulation — a console scroll costs the better
  part of a second — and holding it to the release budget reports load as a
  hang, which it did once.
- `boot-check` has no debug-exit device to report through, so its verdict is
  what the serial log says: a settled marker, and no panic line. Two markers
  kept in step with the kernel by comment, like `READY_MARKER` before them.
- Both new steps were broken before being believed: a panic before settling, a
  panic after settling, and a kernel that never says it settled.

---

## D-66 Setting the interrupt flag with a spin lock held is a checked bug — Status: Accepted (2026-08-23)

**Context.** The self-test hung on roughly one boot in ten, always after the
harness injected a keystroke, and always with the whole machine wedged. It
survived a 400-second budget, so it was not slowness; it happened in release
and debug alike; and disabling the blocked-reader checks made it stop.

Reading did not find it. What did was asking QEMU's monitor where the CPU was:
`RIP` fixed across three samples a second apart, `RFLAGS` with the interrupt
flag clear, and a disassembly showing a `pause`/`cmpxchg` pair against a byte
in `input::EVENTS`. The keyboard handler was spinning on the event queue's
lock, with interrupts off, for ever.

The holder was `read_key`:

```rust
let saved = interrupt::disable();
if let Some(keystroke) = EVENTS.lock().pop() {
    saved.restore();          // the guard is still alive here
    return keystroke;
}
```

The guard is a temporary in the `if let` scrutinee, so it lives to the closing
brace — past the `restore` that sets the interrupt flag. For the few
instructions in between, the queue is locked and interrupts are on. A keyboard
interrupt landing in that window runs `poll`, waits on the same lock, and
waits on a thread that can never resume to release it.

**Decision.** Three things, in order of what they buy.

The window closes: the pop becomes a statement of its own, so the guard is
dropped before the flag goes back.

Setting the flag with a spin lock held becomes a checked bug. `set_flag` is
the one place the kernel sets `IF`, and it now asserts the held count is zero;
`enable_and_halt`, which sets the flag in its own assembly, asserts the same.

Switching threads with one held becomes a checked bug too. `prepare` asserts
before it takes its own lock: a switch would leave the lock held by a context
nothing can schedule, which is the same stranding by another route.

**Rationale.** The rule was already written down — it is the argument
`SpinLock`'s doc comment makes, and the reason `ThreadLock` refuses a caller
holding one. What it lacked was an assert, so a single misplaced statement
could break it and pass the gate a hundred times.

`spinlocks_held` already existed for `ThreadLock`; the count was there and
nothing else asked it. The cost is a relaxed load and a branch on a path that
already does a locked increment.

**Consequences.**

- The reintroduced bug now panics on the first read of every boot, naming
  `interrupt.rs`. What took a hundred boots to see once is deterministic.
- A self-test kernel that panics halts rather than reporting, so `run_selftest`
  watches for the panic marker and fails in seven seconds instead of waiting
  out the timeout.
- On a timeout the harness now asks the monitor for the instruction pointer
  and disassembles around it. Both silent-hang diagnostics this bug needed are
  kept, because the next hang will need them too.
- Three asserts, three breaks: a lock held across a switch, a lock held across
  a halt, and the original bug restored.
- The panic path is exempt from the flag assert and not from the switch one: a
  panic reclaims locks it may find held, and never switches.

---

## D-67 The `unsafe_code` default is deny, and `wost` lifts it in one visible place — Status: Accepted (2026-08-23)

**Context.** D-13 confines `unsafe` to `wost`, and every crate carried the
attribute that says so: thirteen `libs/` crates `forbid`, `kernel` denies,
`wost` said nothing. The gap the 2026-08-23 research pass found is that
`unsafe_code` is *allow*-by-default in rustc, so the discipline was thirteen
separate opt-ins rather than a property of the workspace. A crate added later
that simply forgot the attribute would permit `unsafe` in silence.

**Decision.** `unsafe_code = "deny"` in `[workspace.lints.rust]`, inherited by
every member through the `[lints] workspace = true` they already declare;
`wost` lifts it with a single `#![allow(unsafe_code)]` at its crate root.
`libs/` keeps `forbid`, which is stricter and cannot be lifted locally.
`xtask`, which takes no workspace lints (D-22), carries `#![forbid]` in-source
— found unguarded by the 2026-08-23 audit and closed the same day.

**Rationale.** The default now matches the rule. The exception becomes one
greppable line in the crate whose whole purpose is to be that exception,
rather than an absence of lines everywhere else. Verified in both directions
before the change was made: with the workspace lint, an `unsafe` block in a
crate missing `forbid` is an error; without it, the same code compiles clean.

`forbid` at the workspace level was considered and rejected — `kernel` cannot
use it, because `wost::entry!` expands `#[allow(unsafe_code)]` boot glue there
and `forbid` cannot be lifted by an `allow` even inside a macro expansion.
That is D-21's finding, unchanged.

**Consequences.**

- A new `libs/` crate that forgets `#![forbid(unsafe_code)]` is now caught by
  the workspace rather than by review. Broken once to prove it.
- The `Cargo.toml` comment about per-crate escalation now names both directions
  — the `forbid` in `libs/` and the one `allow` in `wost`.
- `wost`'s crate root gains an attribute that reads as a confession, which is
  the intent.

---

## D-68 A lock guard cannot leave the CPU that took it — Status: Accepted (2026-08-23)

**Context.** `SpinLockGuard` holds an `InterruptState` and restores it on drop;
`ThreadLockGuard` holds a borrow of its lock. Both were `Send`, because
everything they contain is. Nothing in the type system stopped a guard being
moved to another thread and released there.

On one CPU this is latent — there is nowhere else for it to go. On several it
is the bug the whole interrupt-flag design exists to prevent: the flag restored
would belong to the CPU that cleared it, leaving that CPU interruptible inside
its own critical section while another CPU releases a lock it never took.

**Decision.** Both guards carry a private `PhantomData<*const ()>`, which makes
them `!Send` and `!Sync`.

**Rationale.** One line each, no runtime cost, and it closes a class before it
can open. The idiom is already in this codebase — `ThreadContext` is built the
same way (D-62) — and it is the stable-Rust form of what Asterinas states as a
safety principle with `impl !Send`, which needs nightly.

Verified before adoption: `assert_send::<SpinLockGuard<'_, u32>>()` compiled
before the change and is `E0277` after it.

**Consequences.**

- Guards are now pinned to their acquiring context by the type system, not by
  the absence of a second CPU. The SMP debt in D-56 shrinks by one item.
- The two construction sites gain a field. Nothing else changes; no caller
  moved a guard across threads, because there are no threads to move it to.
- Broken once, as D-28 requires: a `Send` assertion on the guard fails to
  compile.

---

## D-69 The lock discipline belongs in the borrow checker — Status: Accepted (2026-08-23), implemented by D-85 with its shape corrected

**Context.** D-66 made "a spin lock is held" a runtime assert, checked at every
point where the interrupt flag is set or a thread is switched. It works, and it
caught the bug it was written for. But it is a runtime check for a property the
compiler could prove, and Phase 5 is about to multiply the number of places it
must hold.

The 2026-08-23 research found that Rust-for-Linux considered exactly this
pattern and rejected it, for reasons that do not apply here: a million-line C
call graph, C callers invoking Rust callbacks, and a maintainer preference
against runtime checks. Their answer was **klint**, an out-of-tree compile-time
preemption-count analyser pinned to a specific nightly — unavailable to a
stable-only project (D-01), and in maintenance mode since 2023.

**Decision.** `SpinLock::lock` will take `&'a mut ThreadContext` and return a
guard borrowing it for the guard's life, so that nothing requiring the token —
that is, anything that may block — can be called while a spin lock is held.
Trap context, which cannot mint a token by construction (D-62), gets a separate
`lock_from_trap()`. The runtime assert stays as the backstop for paths the
borrow checker cannot see.

The change is recorded now and made in Phase 5, alongside the lock work that
phase brings, rather than as a separate pass over 13 statics and 73 call sites.

**Rationale.** This is what klint exists to do, obtained on stable from the
borrow checker — and it is *stronger* in three ways klint's own documentation
concedes: it sees through trait objects and function pointers, which klint
assumes are sleepable; it handles `Drop` and drop flags, which are klint's worst
limitation; and it models `try_lock`, which klint cannot represent.

Verified on the pinned toolchain before being recorded. `lock(cx); drop(g);
sleep(cx)` compiles; `let _g = lock(cx); sleep(cx)` and `let _a = A.lock(cx);
let _b = B.lock(cx)` are both `E0499`.

**Two things testing found that reading did not.**

The guarantee **depends on the guard having a `Drop` impl**. Without one, NLL
ends the borrow at the call and both violations compile silently — the first
attempt at this proof was a false green for exactly that reason. Our guards do
have `Drop`, so the property holds; but the break-test for this ADR is
*delete the `Drop` impl and watch the compile error disappear*, not the more
obvious one.

And the trap-context path is not a compromise. In a handler the flag is already
clear, so "cannot block here" is enforced by the machine; the token would be
proving something already true. `lock_from_trap` can assert `in_trap()`, which
makes the two entry points mutually exclusive by construction.

**Consequences.**

- Two spin locks at once becomes a borrow error. Where nesting is genuinely
  needed, the second acquisition takes the *guard*, and that is where a
  type-level lock rank belongs — pure arithmetic, so `libs/` per D-41.
- Phase 5's lock work is larger by this refactor and smaller by every deadlock
  it forecloses.
- The runtime assert is not removed. Two independent checks is what OSTD ships,
  and the assert covers what the borrow checker cannot.

---

## D-70 SMP: the target model, and the rules that cost nothing now — Status: Accepted (2026-08-23)

**Context.** SMP is Phase 12. The 2026-08-23 research asked whether that is a
foundational mistake and answered: deferring the *bring-up* is correct, and
deferring the *design discipline* past Phase 5 is not — because Phase 5 fixes
the handle table and the queue ABI, the two most SMP-sensitive interfaces a
kernel has.

The decade-long retrofits usually cited were retrofits of large kernels with
frozen ABIs. Small kernels do it in months: Maestro in eleven weeks, Theseus in
twenty-four days, seL4 in one release behind a big kernel lock.

Two of the three reasons to think this untestable turn out to be false. Lock
*ordering* is observable on one CPU — lockdep's own documentation says complex
multi-CPU deadlocks "can be detected on a plain, lightly loaded single-CPU
system as well". The one bug class a single CPU genuinely cannot show is
Store→Load reordering, which is exactly the lost-wakeup re-check `002` already
lists as owed before Phase 5.

**Decision.** No SMP code this side of Phase 12. These rules, which cost
nothing to hold now and are expensive to retrofit:

1. **The target model is one big kernel lock at first bring-up, with per-CPU
   run queues and correct shootdown.** Today's coarse locking is therefore the
   plan, not debt.
2. **Never write "disable interrupts" where "disable preemption" is meant.** We
   currently have no preempt-disable mechanism at all — the only gate on
   preemption is the frame's interrupt flag, so `SpinLock`'s `cli` is doing
   that job. A preempt-disabled-interrupts-on lock is the common case and much
   cheaper; it arrives in Phase 5 with the per-CPU word D-56 anticipates.
3. **No lock guard live across a context switch.** Already enforced by D-66's
   assert in `prepare()`. Recorded here so the SMP rationale is attached to it:
   Redox's September-2025 multi-core hang was fixed with 292 lines of scoping
   for precisely this, and someone weakening the assert later should know it
   guards two things.
4. **`Relaxed` requires justification the way `unsafe` does.** `Acquire` and
   `Release` are identical codegen to `Relaxed` on x86-64, so correctness here
   is free today and unobtainable later.
5. **Interrupt handlers never block.** FreeBSD's opposite answer touched every
   driver.
6. **A full barrier on both sides of the lost-wakeup re-check** — one locked
   instruction on the block/wake slow path.

And three interface rules for Phase 5, from the Scalable Commutativity Rule
(Clements et al., SOSP'13), whose decisive property is that it is analysable
before an implementation exists and before the hardware is available:

7. **The handle table must not promise "lowest free handle."** sv6 measured the
   unordered alternative scaling linearly with *no* single-core penalty.
8. **The queue ABI must not promise global FIFO across submitters.**
9. **Unmap, close and release must not promise global visibility on return.**

**Rationale.** Every item above is either free or already true; what they buy is
that Phase 12 becomes a bring-up rather than a redesign. The mechanical SMP
debts were already tracked in D-45, D-56 and D-60; the semantic ones — lock
ordering, memory ordering, deferred reclamation, interface commutativity — were
on no list, and this is that list.

**Consequences.**

- `THREADS` should be split into a thread table and a run queue: a struct
  boundary now, a scheduler rewrite avoided later. `libs/eevdf` is already
  domain-agnostic and `Entity::place(average, lag, slice)` is the migration
  hook. Redox measured a *global* EEVDF run queue at four cores being three
  times slower than one.
- Rust does not save us here. Converos (ATC'25) model-checked twelve Asterinas
  concurrency modules and found twenty bugs in the safe code.
- `SPINLOCKS_HELD` is still a global, which stays on D-56's per-CPU list.

---

## D-71 An ADR may be Provisional, and a Provisional ADR names what would refute it — Status: Accepted (2026-08-23)

**Context.** This log has two statuses: **Accepted**, and **Verified** for
decisions supported by `003`'s adversarially-verified tier. The 2026-08-23
research pass found that the strongest label was doing work the evidence could
not support. D-13 — the framekernel, the most structural decision in the
project — is marked **Verified** on the strength of a single paper for which
*no independent replication exists*, and whose headline TCB figure does not
survive scrutiny. `008` §7 already recorded that honestly; the ADR did not.

**Decision.** A third status, **Provisional**: accepted and built upon, but
resting on evidence that has not been independently confirmed. A Provisional
ADR must state a **falsification condition** — what would have to be observed
for it to be revisited.

Applied now to three:

- **D-13** (framekernel) — refuted if independent measurement shows the
  discipline's cost materially exceeds the paper's, or if the containment
  property fails to hold at scale in a project not authored by its proponents.
- **D-09** (WASI) — see D-72, which restates it; the watch condition is the
  Component Model reaching Phase 4, or an interpreter shipping support.
- **D-08** (WizFS) — see D-73; refuted if the crash harness proves
  unbuildable before the format is frozen.

**Rationale.** "Verified" should mean verified. Marking a decision Provisional
costs nothing, changes no code, and prevents the specific failure where a
project defends a choice because its own log says the question is settled. The
falsification condition is the load-bearing half: no foundational ADR here
previously stated what would change its mind.

**Consequences.**

- D-13's status changes from Verified to Provisional. This is not a retreat
  from the framekernel — §1 of `010` finds the containment property *better*
  evidenced than before — it is an accurate label on the evidence behind it.
- `003`'s tiers still govern what "Verified" means. Provisional sits below it.

---

## D-72 WASI: `wasip1` is the target, and the Component Model is a watch item — Status: Provisional (2026-08-23), amends D-09

**Context.** D-09 made WASI 0.3 components a first-class app format with
**wasmi** as the runtime. WASI 0.3.0 did ship, on 2026-06-11. Everything else
in that sentence has a problem.

The Component Model is still a **Phase 1 proposal** in the W3C WebAssembly CG,
the lowest active phase of five. "Component Model 1.0" is a roadmap article
whose open workstreams include transitioning the ABI away from eager
allocation, and "simplifying the spec… to reduce implementation burden" — the
Bytecode Alliance saying its own specification is too hard to implement and its
ABI is going to change. WASI 0.3 is implemented by Wasmtime and jco only, and
D-09 already rejects Wasmtime as unusable in-kernel.

And wasmi has no Component Model and no plans for one. Its maintainer, in
September 2025: *"an absolutely huge proposal and therefore an enormous
undertaking for any Wasm runtime to implement… it remains uncertain if a Wasm
interpreter will ever even profit."* Its tracking issue has been open since
January 2024 and was removed from the planned list in February 2025.

The scale, measured: the Component Model spec is ~12,200 lines on top of core
Wasm, and Wasmtime's implementation of it is ~40,900 lines of Rust — larger
than wasmi's entire core interpreter. The published component ecosystem is
three sample HTTP servers and seven micro-utilities.

**Decision.** Core Wasm plus **`wasip1`** on a wasmi-class interpreter, with the
WASI host bindings written directly over native handles. The Component Model
becomes a watch item with a named trigger: **it reaches Phase 4, or an
interpreter ships support.**

**Rationale.** Both reasons D-09 exists survive intact and are delivered by
`wasip1`: WASI's capability model maps onto our handles with no impedance
mismatch — a descriptor *is* a handle grant — and a component compiled for this
OS survives an architecture port. What the Component Model adds beyond that is
cross-language typed *composition*, which is an ecosystem feature, and the
ecosystem is ten components.

`008` §4.5 asked for the "wasmi 1.0" pin to be softened and it was never
actioned; this supersedes that. Runtime selection happens when Phase 7 starts.

**Consequences.**

- Phase 7's WASI work shrinks from "implement the Component Model" — costed at
  zero in the original plan — to "wasip1 host bindings over handles".
- `002-architecture.md`'s app-formats section is restated to match.
- Provisional, per D-71: revisit on the trigger above.

---

## D-73 The filesystem is a destination, not a phase — Status: Provisional (2026-08-23), amends D-08

**Context.** D-08 chose a CoW B-tree with checksums and snapshots, and `003`
set its design rules from bcachefs, btrfs, ZFS and BetrFS. The 2026-08-23
research confirmed almost every rule against primary sources — and then
established what the schedule costs.

bcachefs: announced "more or less feature complete" in August 2015; the
experimental label came off in **June 2026**. Ten years and ten months, by one
exceptional full-time funded engineer — and in between, all 117,000 lines were
removed from Linux 6.18. btrfs: mainline 2009, on-disk format stable late 2013,
and Red Hat *removed* it from RHEL 8 in 2019. ZFS: about five years and a team
growing past twelve paid senior engineers.

And the claim that CoW is the easier road to crash consistency is half false.
OSDI '18: *"btrfs maintains its metadata… in the form of various copy-on-write
B+ trees. This makes achieving crash consistency tricky, as the updates have to
be propagated to several trees."* Of twenty-six crash-consistency bugs studied,
btrfs accounted for twenty-four. CoW makes the *argument* simpler and the
*implementation* harder, and the difference lands in allocation, multi-tree
update ordering, and ENOSPC.

**Decision.** WizFS stops being Phase 6 and becomes a destination reached in a
phase of its own, later. Phases 7–11 run on a **read-only FAT32 or ext2 driver
behind the VFS**. The crash harness is built before the format is frozen.
WizFS's own phase has a definition of done that includes crash injection and a
repair tool.

Two corrections to `003`'s rules while they are open:

- **Strike the blind-upsert ABI rule.** BetrFS states the precondition problem
  and then dissolves it: *"the OS cache of file and directory information was
  able to answer these queries, enabling file creation etc. to run at the full
  speed of upserts."* The technique is sound and ABI-independent; the rule
  solved a problem its own source measured as already solved.
- **The reason to skip parity RAID changes.** ZFS's RAID-Z *eliminates* the
  write hole; it is btrfs's RAID56 and classic RAID5 that carry it. We
  conflated them. The correct reason is that every correct solution needs
  buckets, copying GC and backpointers first.

And one addition that should have been in the rules from the start: **bcachefs
is not a pure CoW B-tree — its btree updates are journalled**, which is what
buys prefix consistency, removes any need for a btrfs-style log tree, and makes
small random metadata updates affordable.

**Rationale.** The design was never the expensive part. Read-only FAT32 makes
the NVMe block path, the VFS, and the buffer plumbing honest at zero
on-disk-write risk — NVMe supersedes the earlier virtio-blk wording because
VMware, the milestone target, has no virtio (`010` §10). FAT32's deeper argument
is that *the oracle already exists*: mount the image on mature operating
systems and let them judge the writer. For a project whose rule is that every
change lands with the check that proves it, that is categorically stronger than
anything buildable for WizFS in one phase.

For calibration, SerenityOS ships ext2 and FAT with hundreds of contributors;
Redox has been committing to `redoxfs` since 2016.

**Consequences.**

- `000-vision.md`'s Phase 6 becomes VFS plus a read-only third-party format;
  WizFS moves to its own later phase.
- **Free, and do it in v0:** put the `snapshot` field in the key from the
  first format version, always zero. Adding a key field later is a format
  break; reserving four bytes is not.
- Not in v0: 128–256K log-structured nodes, and no copying collector.
- Provisional per D-71: refuted if the crash harness proves unbuildable before
  the format is frozen.

---

## D-74 mlibc arrives with the ABI, not after the userland — Status: Accepted (2026-08-23), amends D-10

**Context.** D-10 makes POSIX a userspace translation layer over mlibc, and the
roadmap put it in Phase 9 — after a native userland in Phase 7. The
2026-08-23 research priced that ordering and found it means writing the
userland twice.

It also priced the translation layer itself, which the plan had costed at zero.
Fuchsia ran this exact experiment — capability handles with POSIX confined to a
userspace layer — and reversed: RFC-0082 records that POSIX Lite omits *"parts
of POSIX that imply mutable global state… because Fuchsia is designed around an
object-capability discipline"*, and the answer was Starnix, a Linux kernel in
userspace at ≥15 MB of Rust and 493 syscall-table entries, which implements
`fork`. Plan 9's APE hit the identical wall thirty years earlier — uid/gid,
umask, controlling tty, `link`, `sigprocmask` — and Pike's estimate of the tax
was that *"90–95% of the work in Plan 9 was directly or indirectly to honor
externally imposed standards."*

mlibc's own numbers: 296 sysdep tags, 16 mandatory ("enough for a basic
statically-linked hello world"), and real ports implement 147 to 202. Every one
implements `Fork`, `Sigaction`, `Ioctl`, `Termios`, `Poll` and `Waitpid`.

**Decision.** A stub-heavy mlibc sysdep is written immediately after the Phase 5
ABI stabilises — not to run real software, but to answer *"can a POSIX libc sit
on this ABI?"* while the ABI is still cheap to change. mlibc ships a five-file
skeleton to start from. Pin a release tag, not `master`, per mlibc's own
warning.

**Rationale.** The strongest single correlation in the retrospective evidence is
**conform at the boundary, innovate inside**. Zinnia — one person, started
January 2024 — reached a Wayland desktop in about 2.5 years on mlibc, with a
sysdep of thirteen files. Asterinas reached a Linux-compatible userland in about
three years by adopting the Linux ABI outright. Against that: ToaruOS, fifteen
years solo under a strict no-third-party rule, still not self-hosting; HelenOS,
twenty-plus years, elegant, with no path to third-party software.

This does not weaken the capability ABI. Redox is eleven years in and
*retrofitting* capabilities underneath POSIX, which is strictly harder. It says
the translation layer should arrive early enough to shape the ABI rather than
inherit it.

**Consequences.**

- The question to answer before the queue ABI is fixed: **how does a signal
  interrupt a blocking queue wait?** It is the one that is hardest to answer
  late, and `002` currently does not mention signals at all.
- Adopting mlibc means adopting `abis/linux`, so the *userspace-visible* ABI
  becomes Linux's. The native ABI is unaffected; the claim in `002` that POSIX
  is "only a translation layer" stays true and gets more specific.
- A POSIX conformance lane — os-test and the Open POSIX Test Suite, counts
  recorded over time — arrives with the mlibc slice as a third check location
  beside `libs/` and `selftest.rs`. Both Redox and managarm converged on this
  independently, and it is D-28 extended past the kernel boundary.
- `subprocess` in CPython takes its fork path whenever `cwd` is set, among
  other conditions, so a spawn-only OS needs a `posix_spawn` richer than the
  POSIX baseline. Phase 9's business, recorded here because D-07 is what makes
  it true.

---

## D-75 Phase 11 is an integration milestone, not a browser — Status: Accepted (2026-08-23)

**Context.** `000-vision.md` promises the OS will "browse the web", and Phase 11
is "port NetSurf". The 2026-08-23 research checked what that buys.

NetSurf's last release is **3.11, 2023-12-28**. Its own documentation says it
has *"only primitive and incomplete support for JavaScript, which we disable by
default"* and that full support is *"a long way off"*; its engine is Duktape,
ES5.1-era, and `UnimplementedJavascript.md` lists **1,539** missing bindings.
Its baseline is HTML 4.01 and CSS 2.1.

The porting cost is genuinely modest — a new frontend is about 1,000 lines
minimal, 7,000 basic, and the framebuffer frontend has "no particular operating
system or GUI toolkit requirements". Redox ships it.

**Decision.** NetSurf stays the Phase 11 target. The milestone is described by
what it proves — that the libc, the windowing system, the network stack and TLS
work together against a real third-party codebase — and "browse the web" comes
out of `000-vision.md`.

**Rationale.** It is a good, right-sized integration trophy and it is
demonstrably achievable; it is not web access, and a roadmap that promises the
second while delivering the first will be read as a failure at exactly the
moment it should be read as a success.

The alternatives were weighed. Servo: Redox launched it in October 2025 and
reported it crashes when a second page loads. Ladybird: pre-alpha after 25,000
commits of funded full-time work, targeting a stable release in 2028, requiring
Qt 6.9+, Skia and Mesa, and closed to outside contributions since June 2026.
Nobody solo has a working browser; SerenityOS needed 421 contributors and then
had to spin its browser out into a funded nonprofit.

**Consequences.**

- `000-vision.md`'s destination list says what NetSurf renders.
- The genuinely novel engineering in Phase 11 is coupling NetSurf's `schedule`
  operation to the async ring. Nobody in the surveyed record has done that
  combination, so it is unknown territory rather than a port.
- `links` stays the Phase 9 text-browser milestone. It needs pthreads or fork
  for its async DNS; its DOS port runs fully synchronous and still works.

---

## D-76 The unsafe boundary holds against every safe caller, not just this kernel — Status: Accepted (2026-08-23) (amended by D-77)

**Context.** A soundness review of `wost` asked D-13's question the hard way:
not "does the kernel misuse these APIs" but "*could* any safe caller". Five
holes survived their own refutation, every one a safe path to undefined
behaviour or to hardware state the type system claimed impossible. One was a
plain bug: the fabricated first frame was seven words, so `thread_start`
entered with the stack pointer *at* the page-aligned top — `rsp ≡ 0 (mod 16)`
where the ABI promises 8 — and every non-boot thread ran its whole life
shifted, symptomless only because the kernel builds without SSE. The comment
justifying the seven miscounted the `ret`.

**Decision.** Five contracts hardened, each behind a check that went red first:

- The then-current `mm::read_physical` **copied instead of lending**. The `&'static [u8]` it
  returned could cover memory the kernel itself writes — its own `.data`, the
  boot stack, the framebuffer — and a reference over concurrently written
  memory is an aliasing lie whatever the kind filter says. Volatile reads into
  a caller's buffer are defined for every region the map describes, so the
  kind rules stayed what D-64 set and the lifetime lie was gone. D-77 removed
  this general capability entirely and replaced it with ACPI-only authority.
- `AddressSpace::set` **refuses to re-point a present entry** (identical
  rewrites stay legal — the direct map's outward rounding produces them).
  Silent overwrite let safe code revoke what a `StackArena` or `DeviceMapping`
  is evidence of.
- `DeviceMapping` **carries the root of the space that minted it**, and
  `claim` compares it against the active root. Window addresses collide
  across spaces, so a stale claim from a dropped space could cover whatever
  the installed space put at the same address — two `MmioRegion`s, one device.
- `activate` **walks the whole direct map** in the candidate tables — every
  `is_direct_mapped` region, writable and never-executable at its own frame —
  before loading `CR3`. Every frame access after the switch is raw arithmetic
  against that mapping; the walk is what turns "the kernel surely mapped it"
  into a refusal that is still a `Result`. Stepping by leaf spans keeps it to
  hundreds of walks.
- `switch_from_trap` **accepts only the frame the live handler was entered
  with** (`ForeignFrame` otherwise). The dispatcher records the frame it
  passes, and pointer identity — parked and restored across switches exactly
  like the trap depth — makes it evidence. D-77 additionally made frame state
  private and read-only to safe handlers. `fabricate`'s frame gained its eighth
  word, and the machine now checks a spawned thread's actual stack alignment,
  a claim broken from slice 1 that no register test could see.

Three smaller closures rode along: `boot::enter` is once-only (a second call
could mint a second `Framebuffer` over the same device), the PAT was rewritten
*before* `activate` so the `CR3` load was the flush the manual asks for, and
three `SAFETY` comments now say true things (`MmioRegion: Send` rests on
`!Sync`, not a `&mut` discipline it never had; `breakpoint` asserts the table
it claimed; `apic::write` accounts for its two zero-writes).

**Rationale.** D-13's containment claim is only worth its price if the safe
boundary is sound against adversarial safe code — that is the property the
whole crate split buys. Amends D-45, D-49, D-51, D-54 and D-64, whose
mechanisms changed shape; their decisions stand.

**Consequences.**

- Machine checks at this checkpoint were `a thread starts aligned`, `a
  differing remap is refused`, and `a stale device claim is refused`; the
  direct-map walk was broken once by skipping a region and refused the switch
  on cue. The checkpoint had 75 on-machine checks; D-77 raises the current
  total.
- **Known residue, deliberate:** `AddressSpace::map` still accepts any
  physical address without ownership evidence, so a hostile safe caller could
  alias a live frame into a fresh space before activating it. Closing that
  means mappings take owned frames — D-44's type discipline applied to the
  mapping API — which reshapes `map`'s signature and waits for Phase 5's
  memory objects rather than being bolted on here.

---

## D-77 The completed-phase audit treats green as a claim to attack — Status: Accepted (2026-08-24)

**Context.** A fresh adversarial pass covered every implemented Phase 0–4
boundary: pure arithmetic, boot handoff, interrupt entry and return, physical
memory, page-table construction and activation, device mapping, firmware
parsing, panic recovery, scheduler state, input protocols, and the host gate.
The baseline was green. It nevertheless admitted safe ways to invalidate live
guards, rewrite an interrupt return frame, read arbitrary described physical
memory, alias one device with conflicting cache types, map RAM as MMIO, and
partially mutate page tables before rejecting malformed input. The gate also
accepted ambiguous verdict streams and did not prove that the shipped keyboard
path progressed by interrupt.

**Decision.** The completed phases are hardened as one checkpoint:

- The panic proof and lock-recovery operation are crate-private. `wost` enters
  recovery only through a non-returning function-pointer boundary, and kernel
  panic output uses `try_lock`; no safe caller can recover a lock and then
  resume through a live guard.
- Interrupt handlers receive `&TrapFrame`, whose fields are private and
  read-only. The raw 22-word machine frame has its offsets asserted at compile
  time and its *extent* checked by a live same-privilege probe — the frame base
  plus the words the struct claims must land on the interrupted stack pointer,
  which is the half no compile-time assertion can see. The assembly aligns
  every Rust call explicitly, and `InterruptState` is a single-use,
  non-transferable restoration token.
- Firmware access is an ACPI-only copying capability. RSDP, SDT, pointer-array
  and MADT parsing each enforce both the caller's buffer and the structure's
  declared budget before exposing a prefix.
- Page mapping preflights the whole range before writing an entry. Activation
  proves physical identity for the running code and stack, image permissions
  across both linked and direct aliases, and every stack page and guard in the
  candidate root. Parent tables become user-accessible only when a user leaf
  requires it.
- Device mappings use checked extent arithmetic, reject zero and malformed
  spans, refuse RAM, boot, kernel, ACPI and framebuffer storage, reserve their
  physical interval, and verify every claimed translation. The framebuffer's
  validated geometry is mapped once with WC; PAT entry 1 is changed by a
  reversible read-modify transition. Kernel text is not writable through the
  direct map.
- Hardware-free bounds moved into the relevant `libs/` crates. The pass fixed
  frame-run search at region edges, pixel stride and format validation, slab
  placement overflow, private stack-layout invariants, huge-page PAT encoding,
  canonical mapping ranges, EEVDF's half-range ordering, PS/2 prefix state,
  and the local APIC's legal spurious-vector floor. `libs/mmio-range` now owns
  repeated-access alignment, multiplication and end-bound arithmetic.
- A self-test verdict is exactly one well-formed versioned record with the
  expected non-zero count; missing, duplicate and malformed records fail.
  Serial read errors fail, debug-exit must agree with the record, and the
  shipped boot receives a key and must show IRQ-driven input progress.

**Rationale.** Each defect crossed an abstraction boundary whose prose was
stronger than its evidence. Narrow capabilities, preflighted arithmetic and
physical-identity checks make those claims executable. A second mapping with a
different cache type, a public panic token, a mutable return frame, or a parser
that yields a valid prefix after a corrupt tail cannot be made safe by asking
today's caller to be careful.

**Alternatives.** Keep the historical APIs and add call-site checks (rejected:
another safe caller can omit them); treat the second framebuffer alias as a
performance-only risk (rejected: cache-type conflicts are a correctness
boundary); accept the first valid boot verdict (rejected: trailing or duplicate
records then become invisible); postpone all changes to Phase 5 (rejected: the
next phase would build user authority on unsound kernel authority).

**Consequences.**

- The current boot contract is 79 named machine checks. Every new refusal and
  every new positive path was deliberately weakened once and observed to turn
  the corresponding host or boot check red before restoration.
- **Corrected the same day.** The entry-path check as first written recorded
  the stack pointer *after* the alignment mask, so `alignment == 0` was a
  tautology — D-28's failure mode, in the commit that claimed every check had
  been broken red. It now records where the frame begins and checks its extent,
  and that claim was earned by shortening the declared frame and watching the
  boot report 78 of 79. Two smaller corrections rode along: `Rsdp::parse` reads
  the signature before the revision, so bytes that are not an RSDP are named by
  the field that says so rather than by an undefined revision; and every silent
  refusal on the firmware-read path now says which address it would not read,
  because the narrower authority of this entry makes a routing-less boot the
  likely way it is first met.
- D-31, D-34, D-50, D-64 and D-76 are amended where their original mechanisms
  were too broad. The decisions they protected — terminal kernel faults,
  IRQ-safe locks, WC framebuffer output, bounded firmware input, and a sound
  `wost` boundary — stand.
- Two authority debts intentionally remain for the phase that introduces the
  needed owner types: raw `AddressSpace::map` can still name a physical address,
  and port claims prove exclusivity but not device entitlement. Phase 5 must
  replace the first with owned user-memory mappings; PCI discovery in Phase 6
  must narrow the second for dynamically discovered devices.
- QEMU supplies the automated machine evidence. Phase 3 still lacks its dated
  VMware close record, and no later audit may manufacture one retroactively.

---

## D-78 The handle table is one arena, and a handle is arithmetic — Status: Accepted (2026-08-24)

**Context.** `002-architecture.md` listed "handle table representation and
rights taxonomy" as owed before Phase 5. `010` §3 answered the taxonomy half —
16–24 rights in one `u32`, and a right must name a distinction the kernel makes
about the object rather than a syscall. What remained was the layout and the
lifetime rules, and they must be settled before the first object exists,
because the first caller freezes them.

**Decision.** One global arena, not a per-process array. A slot's base value is
`generation ‖ arena_index`; the value a process sees is
`(base_value << 2 | 0b11) ^ per_table_mixer`. Lookup is arithmetic plus one
ownership comparison. The encode/decode arithmetic goes to `libs/handle-space`;
the kernel keeps the arena and the lock.

- **No lowest-free promise.** The reserved low bits forbid a dense encoding, so
  the promise is unmakeable rather than merely unmade.
- **`HandleTable { table: AtomicPtr<HandleArray> }`** — Linux's
  `files_struct`/`fdtable` split, so readers can go lock-free later without
  touching a call site.
- **The table is a refcounted object**, not an inline field of `Process`.
- **Rights are 16–24 bits of one `u32`**, universal (`DUPLICATE`, `TRANSFER`,
  `WAIT`, `INSPECT`) split from per-type.
- **Type encodings have headroom for 64 object types.** Policy is monotonic and
  frozen — tighten-only, immutable once a process has children — with bits
  encoded so adding a type never renumbers an existing one.
- **Refcounts use `inc_not_zero` from day one.** Close does not promise global
  visibility on return.
- **No revocation.**
- The per-process **quota** D-44 assigned to this doc is answered at this
  layer: a process's kernel memory is bounded by what its handle table can
  name.

**Rationale.** The encoding is Zircon's, and two independent research angles
converged on copying it without sight of each other — one for security, because
generation tagging kills use-after-free on handle reuse; one for scalability,
via the Scalable Commutativity Rule. Zircon's contract commutes *by
construction*. POSIX's "lowest free descriptor" does not: it is an ordering
promise that serialises allocation, and sv6 measured the unordered alternative
scaling linearly with **no single-core penalty** — "at one core, both cases
perform identically." That is the rare case where the scalable choice is free
today, which is why it is taken now rather than in Phase 12.

The rights number is settled by one-directional evidence: seL4 has 4, Zircon
25, CloudABI 31, Capsicum ~81. Capsicum's taxonomy outgrew a machine word and
broke its ABI — `cap_rights_t` is a multi-word struct spending 7 of every 64
bits on metadata, with an `init`/`set`/`is_set` API because two rights can no
longer be OR'd. WASI deleted its rights system entirely in Preview 2 for
creating "a lot of ambiguity about how it mapped to POSIX semantics", which is
exactly what the "a right is a distinction, not a syscall" rule prevents.
managarm, adding fine-grained rights ten years in, landed on ~14.

The refcounted table is a hedge with a named precedent: Zircon's strict
per-process invariant was the first casualty of its own compatibility layer
(RFC-0261 shares handle tables for Starnix). Free now; a redesign later.

No revocation, because the one system that has it paid for it. seL4's recursive
revocation is preemptible and resumable with progress stored in a zombie
capability, and a pre-mitigation worst-case execution time of about one second,
reduced to 436 µs computed and 81 µs observed. Object death plus a strippable
`TRANSFER` right covers the real cases, and if revocation is ever needed, a
per-object **epoch bump is O(1)** — no traversal, no preemption points, no
zombies — and the epoch already exists as the generation counter.

**Alternatives.** A per-process array with dense indices (rejected: it is the
POSIX promise, it serialises allocation, and Zircon does not have one); rights
as a multi-word struct with room to grow (rejected: that is Capsicum's outcome,
not its plan, and the growth is what broke the ABI); revocation now (rejected on
seL4's own numbers); the handle table inline in `Process` (rejected on
RFC-0261); quota at the frame layer (rejected by D-44 — making callers donate
memory is an ABI decision and permanent, while a quota is policy and stays
revisable).

**Consequences.**

- `libs/handle-space` joins the `port_space`/`frame_space` pattern, host-tested
  with doctests. Per D-28 the two checks most likely to be silently unfailable
  are *the invalid value never decodes* and *a reused slot never yields the
  same value twice*; both get broken deliberately.
- The arena lives in `kernel`, not `wost`: it holds no raw pointer and touches
  no hardware, which is `005`'s own test.
- D-70 rules 7 and 9 are discharged for this interface.
- `002`'s open-questions list loses its first entry.

---

## D-79 The queue is one mechanism, and overflow is impossible by construction — Status: Accepted (2026-08-24)

**Context.** The second doc `002` owed before Phase 5. D-06 chose
submission/completion rings and `010` §2 refuted the rationale recorded for
them while confirming the decision, then supplied answers to the four problems
that decide whether a ring ABI is safe.

**Decision.** One ring mechanism, kernel-allocated pages, single owner.

- **Every operation posts exactly one terminal completion, always** —
  cancellation included. Cancellation is advisory.
- **The kernel owns the buffer.**
- **Completion-queue space is reserved at submission.** The CQ is a power of
  two and at least twice the SQ size.
- **Each entry is copied out exactly once and then validated**, enforced by the
  shape of the API rather than by discipline: the validated copy is the only
  thing the kernel can reach.
- **Ports and completion queues are the same object.** Zircon's four semantic
  wins move into the CQ: one-shot arming by default, edge-versus-level as an
  explicit flag, cancellation that purges already-queued entries, and exactly
  one waiter released per entry.
- Ring structures carry **explicit version fields**; the opcode surface stays
  minimal; there are **no privileged opcodes**.
- **No global FIFO across submitters.**
- The rationale recorded in D-06 is replaced by the measured ordering:
  structural asynchrony ≈ 10×, total trap elimination ≈ 1.3× and it costs a
  dedicated core.

**Rationale.** Each rule has a named failure behind it. `-EALREADY` in
io_uring's own manual means "the caller must wait for the canceled request to
post its completion event", so any design where cancellation might suppress a
completion has two exit paths for one operation. Blocking in `Drop` is unsound
because leaking is safe, so a userspace-owned buffer has no sound lifetime;
Windows RIO states the rule kernel-side. io_uring's `CQ_NODROP` took an
unbounded kernel-memory liability for a misbehaving application, while RIO's
reservation-based admission control makes overflow *structurally impossible* —
one of these is a mitigation and the other is a proof. And the double fetch is
not theoretical: CVE-2025-39816 is literally "always use `READ_ONCE()` to read
ring pointers."

Building ports *and* completion queues would give userspace two event planes to
join, which is precisely why io_uring grew `poll_add`, `futex`, `waitid` and
`msg_ring`. A Zircon port already *is* a completion queue whose entries are
events.

The austerity is io_uring's security record read correctly. The ~60% kernelCTF
figure is confirmed and Google still disables io_uring in 2026, but the
statistic is frozen *because* they disabled it. NVD returns 143 io_uring CVEs
and rising — 38 in 2024, 40 in 2025, 36 by partial 2026 — clustering in
**newly added** surface: `zcrx`, provided buffers, `ublk`, passthrough. By
class: 50 race/TOCTOU, 23 use-after-free, 10 refcount. The lesson is feature
velocity on a ring surface, not io_uring the name.

No global FIFO is the Scalable Commutativity Rule again: sv6's mail-server
benchmark shows an ordered socket forces one shared queue while an unordered
one allows per-core queues.

**Alternatives.** io_uring's overflow list (rejected: unbounded kernel memory
for a misbehaving process); userspace-registered buffers (rejected: no sound
lifetime in Rust, because leaking is safe); separate ports and completion
queues (rejected: two event planes is the defect io_uring spent four features
patching); validating in place instead of copying out (rejected: that is the
double fetch, and it has a 2025 CVE); a rich opcode surface (rejected on the
CVE clustering).

**Consequences.**

- Slice 9 writes a TLA+ model before the implementation, per D-84.
- `libs/ring-proto` owns the index arithmetic and the protocol state machine.
- A wait with a timeout and a wait on two queues — both left open by Phase 4 —
  fall out of this mechanism rather than being built separately.
- **If the native ABI is a ring, "observe the syscalls" is not a valid basis
  for auditing, sandboxing or introspection.** ARMO's 2025 "Curing" rootkit did
  command and control without making a single system call, and Falco, Tetragon
  and Microsoft Defender were all blind to it. Every observability mechanism
  this kernel will ever want must be a ring feature, designed in from the
  start.
- `002`'s open-questions list loses its second entry.

---

## D-80 A signal is a completion entry — Status: Accepted (2026-08-24)

**Context.** D-74 named this the question hardest to answer late, and the
reason mlibc moved forward to Phase 5. `002` does not mention signals at all.
The native ABI would never raise it; a POSIX libc raises it immediately.

**Decision.** A signal is an entry posted to the completion queue. A blocking
wait is interruptible because a wakeup is indistinguishable from a completion.
There is no in-kernel signal frame, no kernel signal mask, and no
restart-after-interrupt semantics in the kernel.

**Rationale.** It is the same argument that unified ports and completion queues
in D-79: a second delivery mechanism means two event planes and a join problem,
and the join problem is where the bugs are. Making the signal an ordinary entry
means the interruptible-wait behaviour is a property of the queue that already
has to be correct, rather than a second path that has to be kept correct
alongside it.

**Alternatives.** A kernel-delivered signal frame on the user stack (rejected:
it is a second event plane, it needs its own reentrancy rules, and it puts
kernel-written data on a stack the process controls); a signal-specific wakeup
that aborts a queue wait (rejected: the abort has no completion, which
contradicts D-79's one-terminal-completion rule).

**Consequences.**

- mlibc's `Sigaction` becomes a userspace dispatch loop over CQ entries.
- The POSIX behaviours that assume a kernel-delivered signal interrupting an
  arbitrary instruction — `EINTR` on a blocking call, signal-safe reentrancy,
  a `SIGSEGV` handler that resumes — are either emulated in the libc or
  unimplementable. That is the same cluster Plan 9's APE could not implement
  and Fuchsia excluded from POSIX Lite. **Slice 10's job is to find out which
  half each falls in, and to record the count** rather than discover it in
  Phase 9.
- `002`'s open-questions list loses its third entry.

---

## D-81 The trap entry is FRED-shaped, and FRED is not implemented — Status: Accepted (2026-08-24)

**Context.** FRED shipped on Panther Lake 2026-01-27 and is default-on in Linux
7.1. It replaces IDT delivery and `IRET` with `ERETS`/`ERETU`, replaces IST
with four reentrant stack levels, makes `SWAPGS` a `#UD` in favour of `LKGS`,
and delivers a uniform frame regardless of event — and it is the mechanism that
finally makes supervisor shadow stacks tractable. `010` §10 checked whether
QEMU emulates it: QEMU 11.0.50 lists `fred`, `lkgs` and `wrmsrns` among its
known CPU features and accepts `-cpu max,fred=on`. That moves FRED from
unverifiable to testable, but not to implemented.

**Decision.** Phase 5 builds the *shape* and not the mechanism:

- **Dispatch from a table on a decoded `TrapCause`**, rather than 256 IDT stubs
  that each know their own vector. The decode is pure arithmetic and goes to
  `libs/trap-cause`.
- An **opaque `TrapContext`** at call sites, in place of raw frame field access.
- **`swapgs` behind `arch::percpu`**, never open-coded.
- **`iretq` behind `arch::trap::return_to_user`.**
- **Trap stacks as an enum**, mapped onto IST today and onto FRED's four levels
  later.

Alongside, three entry-path items that are one-liners now and near-impossible
to retrofit gracefully: **`VERW` on exit-to-user** gated on `MD_CLEAR`, an
**eIBRS MSR write at boot**, and **`ENDBR64` on every indirect target in
`global_asm!`** so the naked switch does not break when kernel IBT is enabled.
KPTI is decided *against*, with `RDCL_NO` recorded as the reason.

**Rationale.** Every item is an improvement on its own terms, which is the test
that separates shaping from speculation. The decoded cause is what an aarch64
port would need, which is a strong signal it is the right abstraction rather
than speculative generality. The opaque context is what D-77 already forced for
the frame. `arch::percpu` has to exist for ring 3 regardless. And a table
dispatch is smaller than 256 stubs.

The three one-liners are shaped by cost asymmetry rather than by likelihood:
`VERW` is one instruction on a return path that is painful to modify once
mature, and eIBRS is one MSR write at boot. KPTI is declined because its cost
scales with syscall rate — under 0.5% below 10k/s/CPU, ~5% at 75k, and PCID cut
it from 2.1% to 0.5% — and this kernel's target hardware reports `RDCL_NO`.

**Alternatives.** Implement FRED now (rejected: the transitions are not proven
emulated, the hardware is one generation old, and the shape captures the design
benefit at a fraction of the cost); keep the 256 stubs and grow into them
(rejected: the entry path is rebuilt exactly once, and this is the phase that
rebuilds it); defer `VERW` and eIBRS to a security pass (rejected on retrofit
cost, which is the only reason they are here).

**Consequences.**

- `libs/trap-cause` is a new host-tested crate, checked over the whole vector
  space including the reserved range.
- The existing `global_asm!` trampoline — the most battle-tested assembly in
  the repo, which Phase 4 declined to convert once already (D-51) — is
  converted here, because the table dispatch is what it feeds.
- Whether `syscall`/`sysret` or a trap gate carries the synchronous call set is
  **not** decided here. `sysret`'s non-canonical `RCX` hazard is a real CVE
  class costing a branch on every return, a trap gate costs more per call, and
  the measurement that would settle it is meaningless under TCG. It is decided
  in slice 5 on safety and revisited on real hardware.

---

## D-82 Extended state is eager, compacted, and sized at boot — Status: Accepted (2026-08-24)

**Context.** D-29 deferred XSAVE until there was a scheduler. Ring 3 arrives in
Phase 5, so the decision is due. It is not only a context-switch decision: the
saved size leaks into per-thread stack arithmetic, so it has to be settled
before the stack allocator is.

**Decision.** Eager, compacted, boot-sized, out-of-line, CPUID-derived, with
the arithmetic in a `libs/` crate. (**`libs/xstate` was never written.** The
compacted-offset table it was to hold turned out to be four instructions and a
CPUID read, which live in `wost/src/arch/x86_64/xstate.rs` — the arithmetic
that would have justified a crate was not there. Recorded rather than quietly
dropped, because three lines below still name it.)

- **Eager only. Never lazy.**
- **`XSAVES` → `XSAVEC` → `XSAVE`, and never `XSAVEOPT`.**
- **`XCR0` = x87 + SSE + AVX.** No AVX-512, no AMX.
- **`XstateBuf` out-of-line and growable**, 64-byte aligned, sized at boot from
  the mask — never a fixed array inside the thread struct.
- **Offsets are computed from `CPUID.0DH` into a table** in `libs/xstate` — a
  crate that was never written; see the note above,
  host-tested against synthetic CPUID snapshots. `wost` keeps the four
  instructions and the MSR writes.
- **`#NM` (vector 7) is reserved** for a future XFD-based allocator; nothing is
  built.
- **`+soft-float` stays, and the gate checks that it stays.**

**Rationale.** Lazy FPU restore is CVE-2018-3665 and Linux went eager in 2016;
recording it here is what stops someone re-optimising it back in. `XSAVEOPT`'s
modified-optimisation is a correctness hazard while the init-optimisation that
`XSAVEC`/`XSAVES` provide is the one that pays, and `XSAVES` is preferred even
with no supervisor state today because it shares the compacted format with
`XSAVEC` — and CET's shadow-stack pointers live in supervisor xstate, so
wanting CET later means wanting `XSAVES` now.

Every enabled `XCR0` bit is per-thread memory forever, and AMX's `XTILEDATA`
alone is 8 KiB today and up to 64 KiB architecturally. A fixed array inside the
thread struct is precisely what would make AMX or a future component impossible
to add.

The offset table is not defensive generality. **APX is state component 19 but
sits at offset 960** in the non-compacted format, in deprecated MPX's hole — a
higher index at a lower offset. Linux had to add a dynamic ordering table and
delete its monotonicity assertions in 6.16. A host test against a synthetic
CPUID snapshot with the APX case is the cheapest possible way to not repeat
that.

`+soft-float` is what makes trap entry never touch extended state; only a task
switch does. It is a gate check because it is a build flag nobody would notice
losing.

**Alternatives.** Lazy restore (rejected: CVE-2018-3665); `XSAVEOPT` (rejected:
modified-optimisation correctness hazard); enabling AVX-512 or AMX (rejected:
per-thread memory forever, for no caller); a fixed-size buffer in the thread
struct (rejected: forecloses growth, and the size is not known at compile
time); assuming component offsets increase with index (rejected: APX).

**Consequences.**

- `libs/xstate` was to be a new host-tested crate; the APX snapshot is one of its
  tests, and the break-test is widening `XCR0` by a bit and confirming the size
  arithmetic notices.
- The stack allocator in slice 3 must know the boot-computed size, and must
  also leave a second region slot per thread so CET's shadow stack is not
  foreclosed (`010` §10). Slices 3 and 5 are coupled by this number.
- D-29's deferral is discharged.

---

## D-83 Two rules that cost nothing and keep three options open — Status: Accepted (2026-08-24)

**Context.** `010` §10 surveyed the 2026 hardware security features against
what Phase 5 is about to freeze. Most need nothing. Three — PKS, LAM and its
successor ChkTag — are foreclosed by ordinary-looking code written without
them in mind, and the cost of not foreclosing them is two rules.

**Decision.**

1. **Reserve PTE bits 62:59.** Build nothing on them.
2. **Range-check user pointers. Never test `>> 47 == 0`.**

**Rationale.** PKS is Sapphire Rapids and Intel-only and was **never merged**
into Linux, so building on it would be speculative — but the bits it uses are
in the PTE, and a page-table encoding that spends them is a format change
later. Reserving four bits is not a design.

The pointer rule is about a different kind of breakage. `>> 47 == 0` is the
idiom everyone writes, it is correct today, and it is wrong under LAM, wrong
under 5-level paging, and wrong under ChkTag — all of which put non-address
bits in the top of a pointer. LAM itself is being superseded by ChkTag and is
not worth building for; the rule is worth holding anyway, because a range check
is the same length as the shift and is right in every case. `010` §10's own
phrasing: range-check user pointers, never `>> 47 == 0`.

**Alternatives.** Build PKS support (rejected: never merged upstream, Intel
only, no caller); say nothing and let the idiom appear (rejected: it is the
default thing to write, and it appears once per user-pointer path).

**Consequences.**

- `libs/x86-paging` documents bits 62:59 as reserved.
- Slice 3's user-pointer validation is a range check, and slice 5's
  `copy_from_user`/`copy_to_user` call it before taking the SMAP guard.
- LASS needs discipline only — the split is exactly bit 63, and `#GP` becomes
  the null-deref reporter — which the range check already gives.

---

## D-84 The ring protocol is modelled before it is written — Status: Accepted (2026-08-24)

**Context.** `010` §9 reported that the verification tier is closer than `008`
assumed: Verus runs on this machine on stable today, `shuttle` 0.9 builds
unpinned, and `proptest` builds with two version pins. It also found the wall
and named it precisely — rustup's GNU toolchain ships no `as.exe`, so any crate
reaching `getrandom >= 0.3.2` or `windows-sys >= 0.61` fails to build. And §12
#35 put the sharper point: NrOS paid 13.3:1 and Atmosphere 4.3:1 in
proof-to-code for the same page-table logic, and the difference was the data
layout — a choice made when the ring and handle table are written, not
afterwards.

**Decision.** Two verification tiers are adopted in Phase 5, both scoped to the
ring protocol:

1. **A TLA+ model, checked before the protocol is implemented.**
   `tla2tools.jar` is vendored and checksum-pinned under D-25, and
   `cargo xtask model-check` runs it.
2. **`libs/ring-proto` under `shuttle` and `proptest`** on the host, with
   `proptest` taking `default-features = false` and `getrandom = "=0.3.1"`.

Both tiers get a break-test, and the break is the point:

- The model's break is a deliberately weakened invariant that **must** produce
  a counterexample.
- The concurrency tier's break is a variant using `Relaxed` where the protocol
  needs `Acquire`/`Release`, which **must** fail under shuttle.

**Rationale.** The Scalable Commutativity Rule's decisive property is that it
is analysable "before an implementation exists and before the necessary
hardware is available", and that is the situation this project is in: the
hardware that would show a ring protocol's ordering bugs is in Phase 12. The
one bug class a single CPU genuinely cannot show is Store→Load reordering,
whose shape is exactly the lost-wakeup re-check the ring depends on. A model is
the only thing that can see it today.

Rust does not save us here, and there is a measurement: Converos (ATC'25)
model-checked twelve Asterinas concurrency modules and found twenty bugs — in
the safe code.

The cost is small and already paid for. Java 21 is on the development machine,
so the model tier costs a vendored jar and an xtask. The pins are known and
measured rather than guessed.

**Alternatives.** Write the protocol and test it (rejected: single-CPU testing
cannot observe the one reordering that matters, and the interface is frozen by
its first caller); Verus on the ring (rejected for now: it runs here, but the
proof-to-code ratios above are for page-table logic with a settled layout, and
a protocol still being designed is the wrong first target); nothing, with the
tier recorded as debt (rejected: the whole argument for doing this in Phase 5
is that afterwards the layout is fixed).

**Consequences.**

- `cargo xtask model-check` is a new task. It joins `ci` only if it runs fast
  enough not to damage the loop; if it does not, it is a named task the slice
  runs and this ADR is amended to say so.
- The two `proptest` pins are a recorded tax under D-16. Slice 10's vendored
  clang is what eventually retires them, by supplying the missing assembler.
- A third and fourth check location join `libs/` and `selftest.rs`. `005`'s
  test-placement table grows two rows.
- Verus stays unadopted, with `010` §9's corrected cost on the record so the
  next pass does not re-derive it.

---

## D-85 A lock has two levels, and the leaf is the one a `Drop` can take — Status: Accepted (2026-08-24), implements D-69

**Context.** D-69 decided that `SpinLock::lock` would take `&'a mut
ThreadContext` and return a guard borrowing it, so that blocking with a lock
held, or taking two locks, become borrow errors. It deferred the work to Phase
5 and predicted its shape: 13 statics, 73 call sites, a `lock_from_trap()` for
handlers, and a type-level lock rank in `libs/` for the nestings that remain.
The counts were exact. The shape was not.

**What the doing found.** Three things reading could not have.

*A `Drop` impl cannot receive a token.* `Frame` and `Segment` free themselves
back to the physical allocator when they die, from wherever they die, and
`drop` takes no arguments. `FRAMES` therefore cannot be a token-bearing lock,
whatever the discipline says. The same is true of `GlobalAlloc::alloc`, whose
signature is the language's.

*This kernel holds no lock across a call.* Every one of the 73 sites is
take-mutate-return: Phase 4's scoping — forced by D-66's assert, which refuses
a switch with a guard live — had already produced the property D-69 wanted to
enforce, everywhere. The token has nothing to exclude at 68 of them.

*So the discipline is two levels, not a rank lattice.* A rank with one
non-trivial pair is not an abstraction (`005`, no speculative API).

**Decision.** `SpinLock` acquires in two ways, and the second is the one that
needs justifying.

- **`lock(&'a mut ThreadContext)`** — the default. The guard borrows the token
  for its life, so nothing that can block and no second lock is reachable while
  it is held.
- **`lock_leaf()`** — for an acquisition that *is* the whole operation: take,
  mutate, return, calling nothing. The obligation is on the body, not the
  caller, and it is short enough to read. A leaf may be taken while a
  token-bearing lock is held, which is what makes a nested acquisition sound
  rather than an exception.

`try_lock` and `try_lock_leaf` mirror them. `ThreadLock` gets no token-bearing
form, because its only holders are the two consoles and their work is to format
bytes and return — and a token on `ThreadLock::lock` would infect every `info!`
call site with a parameter, since `log::record` takes the serial console's lock.

**`lock_from_trap()` is not built.** D-69 expected handlers to need their own
entry point. They do not: every acquisition a handler makes is a leaf, so
`lock_leaf` already serves it, and adding a second name for the same operation
would have been API with no caller. The trap-context rule is not weakened —
`ThreadLock` still asserts it, and a handler still cannot mint a token.

**`libs/lock-rank` is not built**, for the same reason. If a genuine multi-level
nesting ever appears, this is where the rank goes.

**The heap stops nesting, which deletes D-47's lock order.** `HEAP` was the one
lock held across a call: `take_slot` refilled from inside the guard, and refill
takes `FRAMES`. It now returns null on an empty free list, and the caller
allocates the frame with the lock *dropped*, re-acquires, and carves. The
window that opens costs at most one spare slab frame when a preemption lands in
it. `HEAP → FRAMES` is therefore no longer an order this kernel has, and
`wost` holds no lock across a call at all: every non-leaf lock in this kernel
is the kernel's own.

**Verified, in both directions.** `let _a = a.lock(&mut cx); let _b =
b.lock(&mut cx)` is `E0499`. `let _g = a.lock(&mut cx); sleep_ms(&mut cx, 1)`
is `E0499`. `let g = a.lock(&mut cx); drop(g); sleep_ms(&mut cx, 1)` compiles.
And D-69's own break-test — **delete `Drop for SpinLockGuard` and watch the
error disappear** — was run: both `E0499`s vanish, because without a `Drop`
impl NLL ends the borrow at the call. The guarantee is `Drop`-dependent, and
that is the break that tests it rather than the obvious one.

**Rationale.** The two-level form gets D-69's property where the property is
live, at the cost of one extra method name, and it is honest about the 68 sites
where the token would have been ceremony. The alternative — a token on every
acquisition — is not available at any price, because `Drop` impls exist.

The heap restructure is the better half of the change. D-47's order was the only
one this kernel had, and it was there because refilling under the guard was
convenient rather than necessary. Deleting an order is worth more than
type-checking it.

**Alternatives.** A token on every acquisition (rejected: `Drop` and
`GlobalAlloc` cannot supply one, so the discipline would have needed an
`unsafe`-shaped escape at its most load-bearing sites); a type-level rank
lattice (rejected as speculative — one pair); `lock_from_trap` as D-69
specified (rejected: no caller); keeping the heap's nesting and expressing it
with a rank (rejected: deleting the order is strictly better than proving it
safe).

**Consequences.**

- `ThreadContext` moves from `kernel::thread` to `wost::sync`, where the
  `in_trap` fact it is built on already lives; the kernel re-exports it, so no
  call site changed name. The blocking API — `yield_now`, `sleep_ms`,
  `sleep_ns`, `park`, `block`, `WaitQueue::wait`, `input::read_key` — takes it
  by unique reference now instead of shared.
- D-47's `HEAP → FRAMES` is retired. `009-multitasking.md`'s lock-order section
  is superseded by `011-userspace.md`'s.
- The runtime assert D-66 added is untouched, per D-69: two independent checks
  is what OSTD ships, and the assert covers what the borrow checker cannot.
- **The compile-time property has no automated check**, and this is a real gap.
  It lives in `wost`, which is cross-compiled, so a `compile_fail` doctest
  cannot run against it (D-41). It was verified by hand against the pinned
  toolchain, above, and re-verifying it is a manual step when the guard types
  change. `libs/` cannot hold it without holding a copy that would drift.
- The machine check the restructure earned is *"a refill costs one frame and
  balances"*. Its other half — that no slot is handed out twice — is checked as
  arithmetic in `libs/slab-layout`, because breaking it on the machine corrupts
  the allocator every later check runs on and produces a general-protection
  fault instead of a red verdict. Phase 4 learned twice that a break which
  crashes proves less than one that fails; this is the first time that lesson
  chose where a check lives.
- Eighty named machine checks.

---

## D-86 Mapping stops being one-way, because the refusal that made it safe is not the signature — Status: Accepted (2026-08-24), amends D-45

**Context.** D-45 made `AddressSpace::map` safe by making it unreachable:
`activate` took the space **by value**, so once the tables were live nothing
held anything to map into. That decision shaped three phases. Thread stacks are
committed at table-build time (D-52) and their count is a real cap; the I/O APIC
window is mapped before the switch and claimed after (D-49); the heap allocates
out of the direct map because it cannot map (D-47). `009` and `007` both record
"how the address space grows" as a decision Phase 5 must take deliberately, and
every slice from 3 onward is blocked on it — a stack cannot arrive with its
thread, and a memory object cannot arrive with its process.

**The finding.** The property that keeps `map` sound was never the signature.
D-77 hardened `set` so that **a present entry accepts only an identical
rewrite**: anything else is `MapError::Conflict`, because "it would re-point a
mapping some handle is evidence of". That refusal is what stops a live mapping
being changed under the running kernel, and it holds whether or not the tables
are active. Taking the space by value was protecting against a hazard the entry
check had already closed.

**Decision.** `activate` takes `&mut AddressSpace` and the caller keeps the
space. The kernel holds its own in `kernel/src/mem.rs`, reached through
`with_active_space`, a token-bearing acquisition (D-85) with the frame
allocation a new table needs as a leaf underneath.

The soundness argument for mapping into a live space is stated in three parts,
each checkable rather than promised:

1. **No live mapping is re-pointed.** `set` refuses any change to a present
   entry, so nothing the running kernel stands on can move.
2. **No invalidation is owed for a new mapping.** A translation that was not
   present was not cached, so a previously absent page needs no `invlpg`.
   Unmapping is the operation that owes a flush, and it does not exist yet.
3. **One `CR3` owner.** A second `activate` is still refused, and the check
   that says so remains — with its rationale corrected, because it used to
   claim it was the reason `map` was safe.

**Rationale.** Removing a constraint by discovering it was already redundant is
the cheapest kind of progress available, and it is only available if the check
that made it redundant was written to be checkable. D-77 wrote that check for a
different reason — an adversarial pass looking for safe callers that could reach
UB — and this phase collected the interest. It is the strongest argument in this
project so far for D-77's posture that green is a claim to attack.

The alternative shapes were both worse. An `ActiveSpace` / `InactiveSpace`
typestate is the obvious Rust answer and `010` §4 names it as a known failure:
*Theseus's `ActivePageTable`/`InactivePageTable` typestate was simply wrong once
two cores existed.* A `wost`-held global reachable through a closure would put a
lock where the kernel's own policy belongs, against D-43.

**Alternatives.** Keep `activate` consuming and add a separate late-mapping API
(rejected: two APIs for one operation, and the second would need the same
refusal to be sound, so the first buys nothing); the typestate (rejected on
`010` §4's evidence); leaving the space in `wost` (rejected on D-43 — which
ranges exist is policy).

**Consequences.**

- D-45's "mapping is one-way" is retired. D-52's stack cap, D-47's direct-map
  heap and D-49's map-before-claim were all *consequences* of it and are now
  revisable — but none of them changes here. Each moves when it has a reason
  of its own, which is slice 3's stacks and slice 7's memory objects.
- **Unmap still does not exist**, and until it does a frame mapped into the
  live space cannot be returned to the allocator: freeing it would hand a live
  mapping's page back. The check this slice adds leaks one frame deliberately
  and says so. The flush token D-70 rule 9 requires arrives with unmap.
- The check `a mapping added after the switch is live` walks the physical
  tables through the direct map, so a `map` that wrote nothing or wrote into a
  table `CR3` does not point at fails it. What it cannot show is the TLB, and
  what it cannot do is touch the page: `kernel` may not contain a raw write
  (D-13), and the safe type for reaching a mapping is the memory-object mapping
  in slice 7. The check grows its second half there.
- Eighty-one named machine checks. The break is to drop the space again, as
  D-45 left it, and the check goes red.

---

## D-87 A mapping carries its evidence: owned frames, or a range the handoff described — Status: Accepted (2026-08-24), discharges D-76's first residue

**Context.** D-76 audited `wost` for what a *safe* caller could reach and found
`AddressSpace::map` taking a bare physical address. D-77 re-checked it, could
not close it, and recorded it deliberately: *"Two authority debts intentionally
remain for the phase that introduces the needed owner types: raw
`AddressSpace::map` can still name a physical address... Phase 5 must replace
the first with owned user-memory mappings."* This is that phase, and D-86 has
just made mapping reachable after activation, which turns the debt from latent
into live: safe kernel code could now map any physical address into the running
tables.

**Decision.** There is no public `map` taking a bare physical address. Three
entries remain, and each carries the evidence its case can actually produce:

- **`map_owned(virtual_base, run: Segment, flags)`** — the caller owns the
  frames and ownership moves into the space. D-44's type discipline applied to
  the mapping API. On failure the run is dropped and its frames go back, which
  is sound because D-77 made mapping preflight the whole range before writing
  any entry: a failed `map` mapped nothing.
- **`map_boot(virtual_base, region: BootRegion, flags)`** — for memory the
  bootloader described. `BootRegion::describe` is the only constructor and
  returns `None` unless the memory map covers the whole range, the base is
  page-aligned, the length is non-zero, and the end does not overflow. It is
  not `Copy`: it is a statement about one range, made once.
- **`map_device`** — unchanged, and already the narrowest of the three (D-49,
  D-64, D-77): it refuses RAM, boot, kernel, ACPI and framebuffer storage,
  reserves the physical interval, and verifies every claimed translation.

**Rationale.** The two legitimate cases are genuinely different and neither can
be expressed as the other. The direct map and the kernel image are *not* owned
frames — nobody allocated them, the firmware and the bootloader placed them —
so demanding ownership there would mean inventing a fiction. What can honestly
be demanded is that the handoff described the range, which is exactly the bound
D-64 gave physical reads: *a bound stated in prose is not a bound*, so the
arithmetic that checks it becomes the type that carries it.

Everything else is somebody's frame, and a frame already has an owner type with
a `Drop`. Moving it into the space is both the authority statement and the
lifetime statement in one move, and it is what gives unmap something to hand
back when it arrives.

**Alternatives.** Keep `map` and add a `# Safety` section (rejected: it is not
an `unsafe fn`, and D-76's whole standard is that a safe caller must not reach
UB however it is called); require ownership everywhere and fabricate `Frame`s
for firmware memory (rejected: a `Frame` whose `Drop` would free bootloader
memory into the allocator is worse than the hole it closes); check the memory
map inside `map` rather than in a type (rejected: the direct map's own
construction would then re-derive per call what one description settles, and
the evidence would not be visible in the signature).

**Consequences.**

- `MapError::Undescribed` names the refusal, with the address it would not map
  — the same courtesy D-77 gave the firmware-read path, and for the same
  reason: the narrower authority makes a legitimate-looking call the likely way
  it is first met.
- `map_stacks` stops leaking its run by hand; it calls `map_owned`, which is the
  one place the `forget` now lives.
- Two checks were rewritten rather than kept: *a differing remap is refused*
  used bare physical `4096` and now presents described regions, and the new
  *an undescribed range cannot be mapped* covers the four ways evidence can be
  absent — undescribed, unaligned, empty, and overflowing. The break is to make
  `describe` assume rather than ask, and it goes red.
- **The second half of D-77's debt is untouched and stays open**: port claims
  still prove exclusivity but not device entitlement. That one needs PCI
  discovery and is Phase 6's, exactly as recorded.
- Eighty-two named machine checks.

---

## D-88 Unmapping returns a token that owns the frames until it has flushed — Status: Accepted (2026-08-24)

**Context.** D-86 made mapping into a live space possible and D-87 made every
mapping carry its evidence, but neither could give a frame back: the check D-86
added leaks one on purpose and says so. Unmap is what closes that, and it is
also where two rules recorded much earlier come due — D-70 rule 9, *unmap must
return a flush token rather than flushing, and must not promise global
visibility on return*, and the standing Rust constraint that **leaking is safe**,
so no soundness argument may rest on `Drop` running.

**Decision.**

`AddressSpace::unmap(virtual_base, len) -> Result<Unmapped, MapError>` takes a
mapping away and hands back a token. The token **owns the frames**, and they
leave it only through `flush()` or the `Drop` that does the same work.

That ordering is the soundness property, not an ergonomic one. A CPU may still
hold a cached translation to a frame whose entry has just been cleared; handing
that frame to the allocator before the invalidation would let it be given away
while a stale entry still reaches it. So the frames are unreachable until the
`invlpg`s have run. `mem::forget` on a token is therefore safe: it costs the
frames and not correctness, which is exactly the property that makes the design
survive Rust's leak rule.

**Provenance lives in the page entry.** `Flags::OWNED` is bit 9, one of the
three the architecture leaves to software (`AVL`, SDM Vol. 3A ch. 4), so the CPU
ignores it. `map_owned` sets it; `unmap` refuses any page without it. A frame
the allocator handed out must come back and one the firmware placed must not,
and the entry is the only thing every path to an unmap can see — a table beside
it would have to be kept in step with every mapping operation, in a kernel where
address spaces are about to multiply.

**What unmap refuses**, each with its own error rather than a shared one: a
hole (`NotMapped`), a misaligned or empty range (`Misaligned`), a large page
(`Unsplittable`), a mapping whose frame is not the allocator's (`NotOwned`),
and frames that are not one ascending run (`Discontiguous`). The whole range is
preflighted before any entry is cleared, so a refusal leaves the space exactly
as it was — the rule D-77 gave `map`, applied to its inverse.

**Large pages are not split.** The direct map is two-megabyte on purpose (D-45),
`010` §8's measurement confirmed that choice, and offering a split here would be
the first step toward losing it. A caller that wants to unmap inside a large
page is asking the wrong question.

**Rationale.** The token shape is what D-70 asked for, and its value is not
visible on one CPU: a caller taking several mappings away pays one invalidation
instead of one each, and when the SMP shootdown exists it has something to
batch. What *is* visible on one CPU is the ownership arrangement, and that is
the half worth getting right now, because it is the half that would be unsound
if guessed.

Putting the frames inside the token rather than returning them alongside it is
the difference between a design that is correct and one that is correct *if
callers are careful*. The same reasoning D-79 applied to ring buffers — the
kernel owns the buffer, because blocking in `Drop` is unsound when leaking is
safe — applies here in the other direction.

**Alternatives.** Flush inside `unmap` and return the frames (rejected: D-70
rule 9, and it forecloses the shootdown batching that is the whole point on
SMP); return the frames beside the token and rely on `Drop` to flush (rejected:
`mem::forget` then hands live-reached frames to the allocator, and leaking is
safe); track ownership in a side table (rejected: it must be kept in step with
every mapping operation across every address space, where the entry cannot go
out of step with itself); split large pages on demand (rejected on D-45 and
`010` §8).

**Consequences.**

- `Flags::OWNED` and `Entry::ABSENT` join `libs/x86-paging`, host-tested: the
  owned bit sits in the `AVL` window, overlaps no hardware flag, and does not
  move the address it decorates.
- `cpu::invalidate_page` is a *safe* function, and the module doc says why: the
  tables are the truth and the TLB is a cache of them, so dropping an entry can
  only force a re-read. The unsound direction is not invalidating.
- The check `a mapping added after the switch is live` gained `OWNED` in its
  expected flags, and found the change by failing — an exact flag comparison
  doing its job.
- Two checks, both broken red: releasing the frames before the flush, and
  assuming provenance instead of reading it. Eighty-four named machine checks.
- **D-86's leaked frame is still leaked**, deliberately: that check maps and
  never unmaps, because what it is about is that the mapping is live. The
  frames it costs are four pages for the life of a boot.
- Still owed by this slice: the thread stack that arrives with its thread and
  leaves with it, which is what turns `STACK_SLOTS` from a cap into a default
  (D-52), and the boot and double-fault stack guards.

---

## D-89 The double-fault handler gets a guarded stack; the boot thread's waits, with its cost — Status: Accepted (2026-08-24)

**Context.** `002-architecture.md`'s security baseline has recorded two
exceptions since Phase 4: *"Two stacks remain unguarded: the boot thread's,
which is the bootloader's, and the double-fault IST stack in `.bss` — both owed
to Phase 5's stack story."* Ordinary thread stacks have had guards since D-52.
The double-fault stack is the worst of the three to leave bare, because it is
the stack the CPU switches to when the kernel is *already* in trouble, and an
overrun there runs into whatever the linker placed next — silently.

Until D-86 there was nothing to be done about it: mapping was one-way, and the
IST stack has to exist before paging does.

**Decision.** After the switch, a guarded region is mapped and the handler is
pointed at it. `segment::set_double_fault_stack` rewrites the interrupt-stack
table entry; the `.bss` array stays where it is and is simply no longer used.

The region's geometry comes from the thread arena's — one slot of the same
size with one guard gap below, placed at `StackLayout::end()`. **This adds no
new hardcoded window**, so it adds no entry to D-46's KASLR ledger, and the
arithmetic is `libs/stack-layout`'s, which already has tests.

`set_double_fault_stack` is **safe**, and the reason is the width rather than
the timing: the entry is a single naturally-aligned eight-byte field, so the
store is atomic on x86-64. A double fault arriving mid-update sees either the
old stack or the new one, and both are mapped and writable — the old one is in
the kernel image and is never unmapped, so there is no window in which the
losing value is invalid.

**The boot thread's stack is not fixed, and this is what that costs.** It is the
bootloader's, and it sits inside the direct map, which is deliberately mapped in
two-megabyte pages (D-45, confirmed by `010` §8). Guarding it would mean either
splitting a large page there — which D-88 refuses on purpose, and which is the
first step toward losing the property `010` measured as worth keeping — or
moving the boot thread onto a stack of its own, which means switching `RSP`
under a running call chain. The second is the real answer and it is not free:
it needs a naked function that never returns, and it interacts with the trap
entry rework slice 4 is about to do. **It is therefore slice 4's, not this
slice's**, and the cost of waiting is one thread's stack overflow going
unnoticed on a kernel that is not yet running untrusted code.

**Rationale.** Half a debt paid with a check that can go red beats a whole one
deferred. And the half taken is the one that matters more: a thread stack
overrun corrupts a neighbour, while a double-fault stack overrun corrupts
whatever is next to it *while the fault handler is running*, which is the
context least able to report it.

This is also the first shipped caller of `with_active_space`, which matters for
its own reason: D-86's capability was until now reached only by a boot-time
check, and an unsafe-boundary change whose only user is its own test is a
change one should be suspicious of.

**Alternatives.** Split the direct map to guard the boot stack (rejected: D-88
refuses splitting, and D-45 plus `010` §8 say keep it that way); move the fault
stack into the thread arena as an extra slot (rejected: `task::init` would then
count it as a thread slot, and the two lifetimes are not the same); leave both
until slice 4 (rejected: this half costs one mapping and closes the worse gap).

**Consequences.**

- `002`'s security baseline loses one of its two exceptions; the remaining one
  now names slice 4 and says what waiting costs.
- The check is structural, as every guard check here is (D-52): the fault is
  terminal, so what is asserted is what the tables say — the interrupt-stack
  entry points at the top of the region, every page of it is writable and never
  executable, and every page below it is absent. The break is to map the region
  starting at the guard, and it goes red.
- `with_active_space` stops being `#[cfg(feature = "selftest")]`.
- Eighty-five named machine checks.

---

## D-90 The gate boots a machine the kernel cannot fully use — Status: Accepted (2026-08-24), extends D-65

**Context.** D-65 made the gate boot three kernels — the self-test image in
release and again in debug for overflow checks, and the shipped image — after an
audit found two configurations the gate never ran. All three boot on `-cpu max`,
where every feature the kernel asks for is present. So every precondition this
kernel checks has only ever been met.

`010` §10 named the specific case: x2APIC is assumed rather than diagnosed, and
`-cpu qemu64,-x2apic` produces a machine without it. D-33 chose x2APIC-only for
a good reason — xAPIC needs page tables that do not exist when the APIC is
brought up — which means there is no fallback to test, only a refusal.

**Decision.** A fourth boot: `cargo xtask degraded-check` runs the shipped image
on `-cpu qemu64,-x2apic` and requires the kernel to **name the missing feature
and still reach its steady state**. It joins `ci` after `boot-check`.

The bar is deliberately not "it works". It is that a machine the kernel cannot
fully use produces a diagnosis rather than a triple fault, and that the
diagnosis is on the serial log where a person will find it.

**Rationale.** A precondition that is only ever met is indistinguishable from
one that is never checked — which is D-28's own argument about checks, applied
to the configuration the checks run in. D-65 learned it once already: the gate
is only as wide as its configuration matrix.

This one is cheap because the kernel already degraded correctly; what was
missing was the evidence. Breaking it confirmed the gap was real: swallowing the
refusal in `start_interrupts` leaves a boot that looks exactly like a healthy
one, and nothing before this would have noticed.

`-cpu qemu64` also strips SMEP, SMAP and UMIP, so this boot exercises D-32's
reporting path at the same time — the reason `MACHINE` uses `-cpu max` at all
(D-65).

**Alternatives.** A self-test check for x2APIC (rejected: the self-test image
boots on `-cpu max`, so the check would assert a feature that is always there —
the unfailable-check hazard D-28 exists to prevent); asserting on the missing
feature and panicking (rejected: a kernel that will not boot on a machine it
could have diagnosed is worse than one that says why); testing every missing
feature (rejected for now: x2APIC is the one with no fallback, and a matrix of
them is a Phase 12 concern when real hardware is in the loop).

**Consequences.**

- The gate boots **four** kernels. D-65's title is now one short.
- `watch_serial` grew an optional second marker, so one reader serves both boot
  checks rather than two diverging copies.
- The break is to swallow the refusal, and the gate reports that the
  precondition is not being checked — naming the marker it wanted.
- Cost: one more QEMU boot in `ci`, about five seconds.

---

## D-91 The speculation posture is read, reported, and decided in a `libs/` crate — Status: Accepted (2026-08-24)

**Context.** D-81 listed two entry-path items as one-liners worth taking now
because they are near-impossible to retrofit: `VERW` on exit-to-user, and an
eIBRS MSR write at boot. It also declined KPTI, recording `RDCL_NO` as the
reason. All three are statements about what the *processor* claims, and this
kernel had no way to ask.

**Decision.** `libs/x86-cpuid` owns the decode: whether the processor reports
`IA32_ARCH_CAPABILITIES` at all, what its bits mean, whether a boot-time IBRS
write is wanted, and what `IA32_SPEC_CTRL` should be left as. `wost` keeps the
`cpuid` and the two MSR accesses. The posture is printed at boot.

Three things the decode is careful about:

- **Silence is not safety.** A processor with no capability word is saying
  nothing, not saying it is immune, and the two are different values rather
  than the same `false`. Every Intel part before 2018 and most AMD parts are
  silent, which is exactly the population where treating silence as `RDCL_NO`
  would skip a mitigation.
- **The MSR is not read unless the feature bit says it exists.** Reading
  `IA32_ARCH_CAPABILITIES` without it is a `#GP`, so this is the one property
  here whose failure is a crash rather than a wrong answer.
- **Enabling IBRS preserves every other bit.** Firmware may have enabled a
  mitigation of its own, and clearing it would be the kernel silently weakening
  the machine it booted on.

Without enhanced IBRS, **nothing is written**. The alternative is per-entry
IBRS, which is a real cost on a hot path and a decision for a phase that can
measure it — `010` §8 is explicit that nothing here is measurable on the current
dev loop, because QEMU/TCG timings are meaningless.

**`VERW` is not implemented, because there is no exit-to-user path yet.** D-81
paired the two, but one is a boot-time write and the other is an instruction on
a return path this kernel does not have until ring 3. It arrives in slice 5 with
`return_to_user`, gated on `MDS_NO` being *absent* — which this crate now
decodes.

**Rationale, and the reason the split is not ceremony here.** QEMU's TCG does
not emulate `IA32_ARCH_CAPABILITIES`. It was asked directly:
`-cpu max,+arch-capabilities` and `-cpu Skylake-Server-v5,+arch-capabilities,+rdctl-no,+ibrs-all`
both boot with the feature bit clear, so the kernel reports "describes no
speculation capabilities" and the IBRS write is never reached. **On this
development loop that branch cannot run at all.**

So the choice was between shipping an unexercised branch in the privileged
crate, or moving every decision it makes somewhere synthetic capability words
can drive it. That is D-30 and D-43's pattern, and `010` §8 asks for exactly it
for the XSAVE table on the same grounds. Six host tests now cover the paths a
boot here never takes.

**Alternatives.** Keep the decode in `wost` and note the gap (rejected: an
untestable branch inside the memory-safety TCB is the worst place to put one);
skip eIBRS until real hardware (rejected on D-81's retrofit argument, and the
report is worth having on its own); treat an absent capability word as
all-clear (rejected: it is the majority case and the wrong answer for it).

**Consequences.**

- `libs/x86-cpuid` is where CPUID-derived arithmetic goes from here. Slice 5's
  XSAVE offset table (D-82) joins it rather than starting a second crate, which
  is the same requirement — synthetic CPUID snapshots, host-tested, `wost`
  keeping four instructions.
- The boot log gains a line naming what the machine claims. On QEMU it reads
  "describes no speculation capabilities", which is true and is the point.
- **This is a decision with no machine check, and that is recorded rather than
  hidden.** The gate cannot reach the enabling path. What it does have is six
  host tests over synthetic words, and the first real hardware boot is what
  will exercise the rest — which is another entry on the list `010` §8 closes
  with: *acquire a netboot-able bare-metal machine.*

---

## D-92 The boot thread leaves the bootloader's stack, because that is the only way to guard it — Status: Accepted (2026-08-24), completes D-12's stack baseline

**Context.** D-89 guarded the double-fault stack and named the one that was
left: the boot thread's. It is the bootloader's, it has nothing below it, and
it sits inside the direct map — which is mapped in two-megabyte pages
deliberately (D-45), confirmed as right by measurement (`010` §8), and which
D-88 refuses to split. So it cannot be guarded where it is. The choice was to
leave it bare or to leave *it*.

**Decision.** The boot thread moves onto a stack of its own, mapped and guarded
like every other, as soon as the tables are live. `kernel_main` ends at that
switch and everything after it runs in `boot_continue`.

The primitive is `task::continue_on_stack`, and it is **safe** because its
argument is the proof. `AddressSpace::map_stack` allocates the frames, zeroes
them, maps them writable and never executable, and then *verifies the guard
pages are absent* — a stack whose guard is mapped is not a guarded stack, and
no caller could tell from the return value, so it is checked where it is known.
The `MappedStack` it hands back cannot be made any other way, and
`is_installed` confirms it belongs to the space this CPU actually walks, since
stack addresses collide across spaces.

What the boot thread carries across goes in a static, not an argument: the old
stack is abandoned, so a pointer into it would point at memory nothing owns.

**The alignment is the part with history.** The continuation is entered by
`jmp`, not `call`, so no return address is pushed — and the ABI wants
`rsp + 8` sixteen-byte aligned at a function's first instruction. The eight
bytes are subtracted at the load instead. That is the same off-by-one word
D-76 found in the fabricated first frame, where it was symptomless without SSE
and invisible to every register check; writing it down here is cheaper than
finding it twice.

**Rationale.** `000-vision.md` claims "ordinary thread stacks guarded" and
`002` carried two exceptions for two phases. Exceptions to a security baseline
decay into the baseline if they sit long enough, and this one had a fix that
was three phases of infrastructure away and is now one function. The
alternative — splitting the direct map to guard a stack — would trade a
measured property for an unmeasured one.

**The check runs on the stack it is about**, which is what makes it more than
structural. The stack pointer under it must be inside the mapped region, so a
switch that silently did not happen fails, where a check reading only the
tables would pass on their strength alone. The break is to decline the switch,
and it goes red.

**Alternatives.** Split the direct map (rejected: D-88 refuses it, D-45 and
`010` §8 say keep it whole); leave the boot thread parked so its stack stops
growing (rejected: it does not guard anything, it just makes an overrun less
likely); make `continue_on_stack` `unsafe` (rejected: `kernel` may not contain
an unsafe block (D-13), so the evidence has to be in the type or the switch
cannot be reached from where it is needed).

**Consequences.**

- `002`'s security baseline has **no stack exceptions left**. Every stack this
  kernel runs on — boot, thread, double-fault — has an unmapped page beneath
  it, and each has a check that goes red without it.
- `AddressSpace::map_stack` is the one place a guarded stack is made now.
  D-89's fault stack keeps its own mapping call for the moment; folding it in
  is tidying, not a decision.
- Three windows are now derived from the thread arena's end rather than
  hardcoded, so D-46's KASLR ledger is unchanged by any of them.
- A kernel that cannot map the stack keeps the bootloader's and says so. That
  is the kernel every earlier phase shipped, so declining costs a guard rather
  than a boot.
- Eighty-six named machine checks.

---

## D-93 Waking a thread from thread context owes it a scheduling decision — Status: Accepted (2026-08-24)

**Context.** `kernel/src/thread.rs`'s module doc states the rule the tickless
design rests on: *"Every path that changes who is runnable ends by arming the
timer for the next moment a decision is due, or stopping it when there is
none (D-57)."* `unblock` changes who is runnable and armed nothing.

Phase 4 never noticed, because the only caller was a wait queue woken from an
interrupt handler — where the trap exit reschedules anyway (D-54). Phase 5's
first kernel object woke a thread from **thread context**, and the woken thread
sat Ready for as long as it took an unrelated interrupt to arrive. On a
tickless kernel with nothing armed, that is unbounded.

**Decision.** `unblock` arms the timer for now when it wakes anything from
thread context. From a handler it does not: the trap exit is about to make the
decision, and arming twice is noise.

**Rationale.** The alternative — making the waker yield — is stronger and
wrong: signalling an event is not a scheduling point the caller asked for, and
a waker that yields cannot signal two things without being descheduled between
them. Arming says "a decision is due" and leaves the policy to the scheduler,
which is where D-53 put it.

The bug was found by a self-check that timed out rather than by reading, and
the check only existed because a new object needed one. Worth recording: the
first caller of an old primitive from a new context is where its unstated
assumptions surface.

**Alternatives.** Yield in the waker (rejected above); arm from trap context
too (rejected: redundant, and the trap path already computes the *right*
deadline rather than "now"); require callers to reschedule (rejected: it is the
rule this module already states, so the primitive should keep it).

**Consequences.**

- `Event::signal` from thread context now wakes promptly. The check that found
  this — *a signal wakes a thread waiting on a handle* — waits for the
  waiter to have actually blocked before signalling, so it cannot pass on a
  thread that never slept (D-49's lesson), and its break is to remove this arm.
- Every future object that signals from thread context inherits the fix rather
  than rediscovering it.

---

## D-94 The mid-phase audit: a refused mapping freed frames it had mapped — Status: Accepted (2026-08-24)

**Context.** D-77 established that a completed phase's green is a claim to
attack. This is the same treatment applied *mid*-phase, to slices 1–4, by a pass
instructed to break them rather than to review them. The baseline was green: 86
machine checks and every host test passing.

It nevertheless found six defects crossing a soundness or evidence boundary, a
tautology in a check written four commits earlier, and six claims in prose that
the code had outrun.

**The worst was mine, and it was introduced by the ADR that was closing the
same class of hole.** D-87 gave `map_owned` this contract:

> On failure nothing was mapped — the whole range is preflighted before any
> entry is written (D-77) — so `run` is dropped and its frames go back to the
> allocator.

**There was no preflight.** `map_range` wrote each entry as it walked, so a
range failing part-way left the earlier pages mapped — and `map_owned`, taking
its own doc at its word, dropped the `Segment` and returned those frames to the
allocator while the space still mapped them `PRESENT | WRITABLE`. The next
allocation hands out memory a live mapping can still write, and a later `unmap`
of that address frees it a second time. Reachable from safe code, in the live
kernel space, through a public `wost` export.

The sentence was inherited from D-77's summary, where it was true of the code
that existed then. Restating a property in a new place is not the same as
holding it, and a doc comment is not a check.

**Decision — the six soundness and evidence fixes.**

1. **`map_range` rolls back.** A failed mapping clears every entry it wrote, so
   "nothing was mapped" is true rather than claimed. Tables allocated on the way
   are left: they are zeroed frames pointing nowhere, and reclaiming one means
   proving no sibling survives.
2. **`unmap` refuses non-canonical addresses.** A walk masks bits 48 and above,
   so the low-half alias of a higher-half address reached the same tables — and
   the `invlpg` that followed named the non-canonical form, which invalidates
   nothing. Clearing a live mapping through its alias and handing the frames
   back left a stale writable TLB entry: precisely the hazard `Unmapped` exists
   to prevent.
3. **`set_double_fault_stack` takes `&MappedStack`.** It took a bare `u64` and
   checked only alignment, so any safe caller could point IST1 at a `Vec`, at
   kernel data, or at a hole — on the one stack that is used only when the
   machine is already in trouble. Four hundred lines away, `continue_on_stack`
   already refused to stand on anything without evidence.
4. **The store is the eight-byte entry, not the whole segment.** Its SAFETY
   comment argued from the field's width while the code wrote the entire
   `TaskState` — a memcpy, observable torn. `TaskState::interrupt_stack_offset`
   now names the offset, host-tested against SDM figure 8-11.
5. **`guard_the_fault_stack` goes through `map_stack`.** It mapped by hand and
   never verified the guard; only the self-test build checked it, while `002`
   claimed every stack had one.
6. **`map_owned` maps four-kilobyte pages only.** A large page carrying `OWNED`
   is one `unmap` must refuse for ever, since it does not split (D-45) — an
   unreachable leak. A few more entries is the cheaper mistake.

Two smaller ones rode along: `cpuid(7, 0)` is read without a max-leaf guard
(a `#GP` on pre-2012 hardware), and `continue_on_stack` abandons a stack without
asserting no lock guard is live on it — the rule `prepare` already enforces for
the analogous case.

**Decision — the checks.** *the double-fault stack is guarded* ended on
`stack_top() >= layout.base()`, which is true of every layout ever built:
D-77's own corrected failure mode, reappearing in the commit that added it.
*unmapping refuses what it does not own* claimed to test the direct map's large
pages but used the kernel's linked text address, and accepted either of two
errors so it could not say which mechanism fired — it now checks six distinct
refusals, and the direct-map address was *probed* rather than assumed, because
the first two megabytes are mapped in small pages. *an unmapped range returns
its frames* counted free frames before a mapping that could allocate a page
table, so it passed only because an earlier check had already built that table.
And `libs/slab-layout`'s new test asserted `n*b >= n*b`, true by algebra; it now
compares against counts and offsets transcribed by hand.

**Decision — the prose.** `011`'s lock-order section claimed taking two spin
locks is `E0499`. It is not: `ThreadContext::claim` mints a token wherever the
interrupt flag says thread context, so a second one can be minted inside a live
guard. The compile-time half is a strong default — one visible, greppable line
to defeat, audited like an `unsafe` block — not a proof, and the section now
says so. It also referenced `libs/lock-rank`, which D-85 explains was never
written. `009` claimed this kernel holds no lock across a call inside `wost`;
`mm::with_memory_map` does. Four stale `SAFETY` comments and module docs in
`paging.rs` still said nothing unmaps.

**Rationale.** Every finding was reachable by reading the code against its own
documentation, and none of it was found by the gate — because the gate checks
what the checks check, and the checks had drifted toward restating the
implementation. That is the argument for the adversarial pass being a *pass*
rather than a review: the instruction was to break it, and being balanced would
have produced none of this.

**Consequences.**

- *a refused mapping leaves no trace* is the check the first defect earned, and
  it was verified by reverting the fix and watching it go red — the only way to
  know a check would have caught the bug it was written for.
- Ninety-four named machine checks.
- **Three findings are recorded and not fixed**, because each is a design
  decision rather than a repair: the closure-under-guard hole
  (`with_memory_map`, `with_io_apic`, `with_active_space`) needs a guard type
  that forbids calls; `mem::forget` on a `SpinLockGuard` strands the interrupt
  flag, which `Unmapped`'s own doc says no soundness argument may rest on; and
  `MappedStack` can be invalidated by `unmap` while the evidence is still live.
  They are in `011`'s open list.
- The lesson worth keeping: **a property restated in a second place is not a
  property held in both.** D-87's contract was true where D-77 wrote it and
  false where D-87 repeated it, and nothing but reading the two together would
  have said so.

---

## D-95 The second audit: a fix that introduced undefined behaviour, and a wait that could sleep for ever — Status: Accepted (2026-08-24)

**Context.** A second adversarial pass, over slice 6 and over D-94's own
repairs. The baseline was green — 94 machine checks, every host test passing —
and it found eleven defects, three of them worse than anything in the first
pass.

**The worst was in a D-94 fix.** D-94 replaced a whole-`TaskState`
`write_volatile` with a targeted eight-byte store, arguing from the field's
width that the result was atomic. But `TaskState` is `#[repr(C, packed(4))]`
and `IST1` sits at offset `0x24` — four-mod-eight. In a segment on an
eight-byte boundary that field is *four*-byte aligned, so the store was
**misaligned, which is undefined behaviour**, and the atomicity the argument
rested on held by the linker's choice rather than by anything stated. The code
it replaced was correctly aligned and merely torn-observable. A fix made it
worse in the one dimension the fix was about.

The repair is to place the segment four-mod-eight, with four bytes of padding
in front, so that `0x24` lands on a boundary. `x86-tables` now pins both facts:
the offset is `0x24`, checked against `offset_of!` rather than against itself,
and it is four-mod-eight, which is *why* the padding exists.

**Second: `Event::wait` could sleep for ever, and three docs said it could
not.** The shape was check-then-register — read the condition, then call into
the wait queue, which registers and blocks. A signal landing in the gap woke an
empty queue, and the waiter then blocked with nothing left to wake it. The
claim that a sticky signal covered this was wrong in a way worth naming: the
state can only be re-read once a block *returns*, and it returns only on a
wake, so stickiness rescues the one case that stops arising as soon as an idle
thread exists.

`WaitQueue::wait_unless` replaces it. Registration and the re-check happen
under the same queue lock a waker must take, so the two are one step and a
signal cannot land between them. It also clears the registration bit on the way
out, which the old path never did on a non-wake return — a stale bit names a
thread that is no longer waiting there.

**Third: D-94's rollback cleared live page-table entries without invalidating
them.** `undo` runs against the *active* space, and `map_owned` hands the
frames straight back to the allocator afterwards — the same stale-TLB hazard
D-94 fixed for `unmap`, arriving by the other door. One `invlpg` per cleared
entry, and only when the space is the installed one.

**The rest.**

- **`ARENA` held its leaf guard across allocation.** `Vec::push` in the grow
  path and `Vec::new` in `Drop` both call the allocator, rebuilding
  `ARENA → HEAP → FRAMES` — the order D-85 deleted, reintroduced without being
  written down. Growth is now reserved outside the lock; `Drop` sizes its
  buffer from `live` before taking it.
- **`insert`'s recovery path was a guaranteed no-op.** It called `close(0)`,
  and zero can never decode, so the slot it claimed to free stayed occupied.
- **`Mixer` gave table zero the identity.** The kernel derived it as
  `id * 0x9E3779B9`, which is zero for the first table — so the kernel's own
  handles were unmixed and its first one was the literal integer three. And
  the crate's doc claimed a small integer "is never a valid handle", which is
  false for any value with the reserved bits set. `Mixer::for_table` adds an
  odd constant after the multiply; the sentence is corrected.
- **Rights were never checked against object type.** D-78's one rule is that a
  right names a distinction the kernel makes *about that object*, and the first
  object shipped with `READ`, `WRITE`, `EXECUTE` and `MAP` granted on an event,
  which has no pages. Worse, the check named `rights are checked and only
  narrow` used that grant as its fixture. `Rights::are_meaningful_for` and a
  per-type mask make the rule enforceable; `insert` refuses the rest.

**Three checks could not fail.**

- *a handle does not cross tables* accepted any of three errors, and the one it
  actually got was `Stale` from the arena's bounds check — because two mixers
  differ, so a foreign handle decodes to an index that is not in the arena and
  never reaches the ownership comparison. **Deleting that comparison left the
  check green.** It now forges a handle with the other table's mixer, and
  breaking the comparison turns it red.
- The `x86-tables` layout test wrote a value into a zeroed array at the offset
  it was testing and read it back from the same place — an identity that would
  pass under any layout. It now checks the transcribed offset against
  `offset_of!`.
- `a_malformed_value_never_decodes` skipped, with a `continue`, exactly the
  values where its property was interesting.

**And one break behaved badly.** The first attempt at a check for the wait
window *hung the machine* instead of failing: its predicate signalled the
event, which takes the queue lock the predicate already runs under. The second
attempt passed under the broken code, because `Event::wait`'s outer check
short-circuits and never reaches `wait_unless`. The third runs the wait on a
spawned thread and calls `wait_unless` directly, so a lost wakeup is a timeout
in the *observer* — red rather than hung. Three attempts to make one check able
to fail honestly.

**Rationale.** Two things this pass says that the first did not.

The first is that **a fix is not exempt from the standard it was written to
uphold**. D-94's TSS repair was written to remove a torn store and introduced
undefined behaviour, in a `SAFETY` comment that argued for the opposite. It was
green, checked, and wrong.

The second is that **the race a check steps around is the race nothing tests**.
The existing event check waits for the waiter to have blocked before
signalling, which was right for D-49's reason and left the actual window
unexercised. One CPU cannot drive it, and that is D-70 rule 6's whole point —
which is why slice 9's model, not a self-check, is what will settle it.

**Consequences.**

- Ninety-six named machine checks. Six new or repaired ones, each broken.
- `011-userspace.md`'s lock-order section gains `ARENA`, which slice 6 added
  and recorded nowhere.
- **Two findings are carried, not fixed.** `undo` can still clear an entry that
  existed before the call, where a range overlaps an identical pre-existing
  mapping — only the direct map's outward rounding produces that, and a failure
  there aborts the whole space, but the next caller that recovers from a
  `map_range` error will meet it. And `NotYours` versus `Stale` is an occupancy
  oracle over a global arena: a table can learn which slots another holds.
  Zircon collapses both to one error for that reason, and this is a decision
  for the phase that has a syscall to surface them through.

---

## D-96 Ring 3 arrives through a gate of its own, and `enter_user` checks rather than trusts — Status: Accepted (2026-08-24) (its one-thread limit lifted by D-99; corrected and hardened by D-101)

**Context.** D-81 shaped the entry path and left one question open on purpose:
whether the synchronous call set arrives by `syscall`/`sysret` or a trap gate.
Its reasoning was that `sysret`'s non-canonical `RCX` hazard is a real CVE class
costing a branch on every return, a trap gate costs more per call, *and the
measurement that would settle it is meaningless under TCG* (`010` §8).

**Decision. An interrupt gate at DPL 3, and the reason is not performance.**
(Called "a trap gate" throughout the first version of this entry, which was
wrong twice over: the mechanism chosen is an *interrupt* gate — the flag is
cleared on entry — and `libs/x86-tables` says a trap gate "is deliberately
unavailable". Corrected by D-101.) `syscall` needs
`swapgs`, `swapgs` needs a per-CPU block, and a per-CPU block reached through
`gs:` is a second unproven mechanism in the same commit as the privilege
boundary. A gate reached through the interrupt table needs none of it: the CPU
loads `RSP0` from the task-state segment itself. So the boundary is established first, on the
simplest mechanism that can hold it, and `syscall` becomes an optimisation with
something working to compare against — on hardware, where the comparison means
something.

Exactly **one** gate admits ring 3, `0x80`, and it is named rather than
defaulted. Every other vector raises a general-protection fault when user code
says `int`, because a device's line is not a service.

**`enter_user` is safe because it checks.** `iretq` will send the CPU to an
unmapped address in a privilege level that cannot report it, so every
precondition is walked first: the tables are live; the entry is present, `USER`
and executable; the stack is present, `USER`, writable and *not* executable;
the stack pointer is sixteen-byte aligned; and `RSP0` names somewhere for the
trap to land. A caller that gets any of them wrong is told. Breaking the code
page's `USER` bit produces *"0x400000000000 is not executable from ring 3"* and
a red check, where without the walk it would produce a triple fault.

The frame it pushes sets the interrupt flag, because a ring-3 thread that
cannot be preempted owns the machine. Every general register is zeroed on the
way out: a register left set is a kernel pointer handed to user code.

**The descriptor order is fixed now.** `USER_DATA` sits *before* `USER_CODE`,
which looks backwards and is what `sysret` requires — it derives `CS` from
`STAR + 16` and `SS` from `STAR + 8`. Nothing uses `sysret`, and the order is
settled anyway, because moving a descriptor later means moving every selector
that names it.

**Rationale.** The evidence this produces is the point. The check reads the
code selector the *CPU* pushed when it took the interrupt — a ring-3 value
there cannot be manufactured by a kernel thread saying `int 0x80`, because the
number is the hardware's account of where the caller was. Alongside it: the
stack selector is ring 3, the stack pointer is in the low half the kernel never
maps, the vector is the one gate, and no other gate has DPL 3.

And when the gate is closed, the machine says so precisely:
`general protection fault (vector 13, error 0x402) at 0x400000000000, cs 0x33,
rsp 0x300000001000`. Error `0x402` is IDT entry `0x80` refused; `cs 0x33` is
selector 6 at RPL 3. The hardware confirming every part of the boundary at
once.

**The first program is four bytes and deliberately not an ELF.** `int 0x80`
then `jmp .`. Slice 7 brings `libs/elf64`, an initrd and `spawn`; this is the
layer beneath, and keeping them apart means a failure here is a failure of the
boundary rather than of a parser. The trailing jump is unreachable — the caller
is retired from the far side — but a program whose last instruction falls
forwards executes whatever the page holds, and stopping deliberately is cheaper
than reasoning about that.

**`retire_from_trap` is `park`'s trap-side twin.** A thread that has crossed
into ring 3 cannot `park`: that wants a `ThreadContext`, and trap context
cannot mint one by construction (D-62). So the syscall handler marks the caller
blocked with nothing that will ever wake it and switches away at trap exit,
exactly as preemption does.

**Alternatives.** `syscall`/`sysret` first (rejected: it needs `swapgs` and a
per-CPU block, which is two unproven mechanisms in the commit that establishes
the privilege boundary — and the measurement that would justify the complexity
cannot be taken here); a gate at DPL 0 with the kernel entering ring 3 some
other way (rejected: there is no other way back); making `enter_user` `unsafe`
(rejected: `kernel` may not contain an unsafe block (D-13), so the checks have
to be inside or the boundary cannot be crossed from where it is needed).

**Consequences.**

- `Frame::write_at` and `Segment::write_at` are the one way safe code puts
  chosen bytes in a frame. They exist so a page can be filled *before* it is
  mapped read-only and executable, rather than mapped writable and demoted —
  W^X is not relaxed for user memory, it is the reason user memory exists.
  Slice 7's loader is their second caller.
- **One thread may be in ring 3 at a time.** `RSP0` is set once, to a stack of
  its own, rather than to the calling thread's. Per-thread `RSP0`, updated by
  the scheduler on every switch, arrives with the processes that need it in
  slice 7. This is a limitation, not a design.
- The retired thread keeps its slot and its two mapped pages for the life of
  the boot. Nothing reclaims a retired thread yet; `spawn`'s counterpart is
  what will.
- `mod user` is `#[cfg(feature = "selftest")]`, like `handle` and `object`:
  built and proven, and nothing ships it until there is a program to run.
- Ninety-seven named machine checks. The break that goes red is the code page
  losing its `USER` bit; the break that goes *bang* is closing the gate, and
  the fault it produces is better evidence than any assertion.

---

## D-97 The user copy is the boundary, and its window is three instructions wide — Status: Accepted (2026-08-24)

**Context.** D-81 named `stac`/`clac` as *"a scoped RAII guard; it **is** the
user-copy ABI"*, and listed SMAP among the features to shape before ring 3. Ring
3 exists now (D-96), so there is user memory to copy and a reason to copy it.

**Decision.** Two functions, `copy_from_user` and `copy_to_user`, and nothing
else in this kernel may open the SMAP window. The guard is a private RAII type
whose whole life is one `copy_nonoverlapping`: an open window is the CPU
agreeing to dereference whatever a user pointer happens to be, and the shorter
it is, the smaller the class of bugs that can be an escalation rather than a
fault.

**Range-checked, never `>> 47 == 0`** (D-83). Every page of the range is walked
before a byte moves — present, `USER`, and writable where the copy writes — and
a range that starts inside user space and ends outside it is refused, which the
shift cannot see. A user pointer naming kernel memory, or nothing, is a refusal
rather than a fault: a fault here is terminal (D-31), and "the program passed a
bad pointer" is not a reason to stop the machine.

Where SMAP is absent the window does nothing, because `stac` is a `#UD` on a
processor without it — which the degraded boot (D-90) exercises.

**The release build caught a bug the debug build could not.** The first version
gave `stac` and `clac` the options `nomem` and `preserves_flags`. Both are
wrong, and wrong in a way that only optimisation reveals:

- **`nomem` is a lie about the instruction's purpose.** `stac` changes whether a
  *later memory access faults*. Telling the compiler it touches no memory lets
  it move the copy out of the window — which it did, in release only, and the
  machine took a protection violation reading the page it had just been given
  permission for.
- **`preserves_flags` is a lie about its name.** The flag it sets is
  `EFLAGS.AC`.

The debug build passed. D-65 made the gate boot both profiles after an audit
found configurations it never ran, and the reason recorded then was overflow
checks; this is the same lesson arriving through a different door.

**Rationale.** The refusals are the half worth checking, and the check drives
all five: kernel memory named as the user's, a range straddling the end of user
space, a range that wraps, an unmapped page, and a write to the program's own
code page — which is user-readable, executable and read-only, because W^X holds
for user memory too.

Two of those breaks behave differently and both are informative. Replacing the
range check with `>> 47 == 0` turns the check red on the straddling case, which
is precisely the failure D-83 predicted when it forbade the idiom. Removing the
writability check produces a page fault instead — the hardware confirming the
mapping really is read-only, which is better evidence than an assertion and a
worse break, because a crash says less than a failure (D-28).

**Alternatives.** A public window type callers open around their own accesses
(rejected: it is the ABI, and an ABI with two users is an ABI with two
disciplines); trusting the caller's bounds and letting a bad pointer fault
(rejected: terminal, and the program is the untrusted party); a shift instead of
a range check (rejected by D-83, and the check that catches it is now in the
tree).

**Consequences.**

- Ninety-eight named machine checks.
- The copy reads the very instructions the CPU executed in ring 3, through the
  *user* mapping — stronger than a round trip through memory the kernel also
  wrote.
- `USER_END` is where user memory stops, named once. Nothing computes it.
- Still owed: a fault *during* a copy is terminal, where a real kernel would
  recover and return an error. That needs a fixup table keyed on the faulting
  instruction pointer, and it needs the trap dispatch D-81 built to be able to
  consult one. It is not needed until a program can pass a pointer that races
  its own unmapping, which is slice 7's problem at the earliest.

---

## D-98 The return to ring 3 clears the buffers, and is not gated on a bit this machine cannot read — Status: Accepted (2026-08-24)

**Context.** D-81 listed `VERW` on exit-to-user as one of two one-liners worth
taking before ring 3 exists, *"gated on `MD_CLEAR`"*, on the grounds that an
instruction on a mature return path is painful to retrofit. Ring 3 exists now
(D-96), so the path exists to put it on.

**The gate D-81 specified would make it unreachable here.** QEMU's TCG reports
no `IA32_ARCH_CAPABILITIES` (D-91) and no `MD_CLEAR` — asked directly, with
`-cpu max,+md-clear` and `-cpu Skylake-Server-v4,+md-clear`. Gating on that bit
would have left the instruction never executed on the development loop, in the
same untestable state eIBRS is still in.

**Decision.** The flush is armed **unless** the processor reports `MDS_NO`.

`verw` exists on every x86-64 — it is an old instruction that verifies a
segment for writing — and `MD_CLEAR` says only whether it *also* clears the
microarchitectural buffers. So a machine whose capability word cannot be read
gets the instruction rather than the assumption, which is the direction to be
wrong in: the cost is a few cycles on a return, and the alternative is skipping
a flush on a machine that needed one. `MDS_NO` is the single claim that makes
it pointless, and it is honoured.

The branch is in the entry stub's tail, after the frame is restored: the saved
`CS` is tested for ring 3, and the flush is skipped for a return to ring 0 —
which is every return this kernel makes except the ones that matter. The armed
flag is a plain byte, read with one instruction and written once at boot,
before any user code exists.

**Rationale.** The policy is what makes this checkable at all, and that is the
whole argument. Under D-81's gate the instruction would have been shipped
unexecuted, verified by reading; under this one it runs on every ring-3 return
the self-tests take, and the ring-3 crossing check passes *through it*.

What no check here can show is that the flush flushes — nothing observable
changes, and on this machine it very likely buys nothing, which the boot log
says out loud. What it shows is that the branch is armed and that a return was
taken through it, which is the part that could crash and the part a later edit
could silently remove.

**Alternatives.** Gate on `MD_CLEAR` as D-81 specified (rejected: unreachable
here, and the failure mode of the gate being wrong is the mitigation silently
absent); flush unconditionally including returns to ring 0 (rejected: it is
every return this kernel makes, for no boundary crossed); use the software
sequence Linux falls back to without `MD_CLEAR` (rejected: it is long, it is
processor-specific, and nothing here can measure whether it helps).

**Consequences.**

- Ninety-nine named machine checks. The break is to leave it disarmed.
- The boot log names both the decision and its limit: *"buffers cleared on
  every return to ring 3 (md_clear unreported, so the flush may buy nothing
  here)"*. A machine reporting `MDS_NO` says so instead.
- D-81's second one-liner, eIBRS, remains armed-but-unexecutable here (D-91).
  The difference is that `VERW` had a policy that could be moved off the
  unreadable bit and eIBRS does not: enhanced IBRS genuinely requires the
  capability word to know it exists.

---

## D-99 The ring-0 stack moves with the thread — Status: Accepted (2026-08-24), lifts D-96's limitation

**Context.** D-96 set `RSP0` once, to a stack of its own, and wrote the
consequence down rather than hiding it: **one thread may be in ring 3 at a
time**. A second would have taken its trap on a stack the first was standing
on, and the corruption would have been silent until the two frames overlapped.

**Decision.** `RSP0` is written by `prepare`, the one function every context
switch passes through, from the stack top the task table already holds for the
slot being switched to. A thread carries its landing with it.

The boot slot has no arena stack and its zero is skipped — it is also the one
thread that cannot be in ring 3, being the one that put the others there.

**The evidence is not a type, and that is deliberate.** `set_kernel_stack` takes
a `MappedStack` because D-95 refused a bare address from outside the arch
module. The switch has no `MappedStack` to offer: these are arena stacks, mapped
before activation by `map_stacks`. What stands behind them instead is
`task::init`, which walks every stack and every guard in the arena and refuses
to fabricate a thread if one disagrees (D-52). So `write_kernel_stack` is
`pub(super)`, reachable only from inside the module where that walk happened,
and the bare-address entry stays refused everywhere else.

**Rationale.** Putting it in `prepare` rather than in the kernel's scheduler is
the whole point. There are three ways into a switch — `switch_to`,
`switch_from_trap`, `exit_to` — and a scheduler that set `RSP0` at two of them
would leave the third as a silent privilege bug. `prepare` is where the slot is
validated and `CURRENT` is stored; adding one more thing that must be true of
the incoming thread belongs beside them.

**Consequences.**

- One hundred named machine checks. The new one spawns a second ring-3 thread
  and requires that its trap landed somewhere other than the first's, which a
  single shared stack cannot satisfy. Breaking the write turns *three* checks
  red — the two crossings and the buffer clear — because `enter_user` refuses
  when `RSP0` is zero rather than entering ring 3 and faulting (D-96).
- The dedicated user-trap stack window is gone, and with it the fourth layout
  derived from the arena's end.
- **This is not isolation.** The two programs share one address space and one
  set of tables; what is separate is the *kernel* side of two simultaneous
  crossings. A second address space is the rest of this slice, and until it
  exists "another program" means "another pair of addresses".
- Eager XSAVE is now reachable and still absent: two threads can be in ring 3,
  so two can have floating-point state to lose. Nothing uses it yet — the
  program is four bytes — but the gap is real from here rather than
  hypothetical, and it arrives with the processes in the rest of this slice.

---

## D-100 The program comes off an initrd, and the loader refuses what it may not map — Status: Accepted (2026-08-24)

**Context.** D-96 put four bytes of machine code in a page and jumped to them.
That proved the privilege boundary and nothing about loading. The roadmap's
Phase 5 wants ELF loading and an initrd, and `libs/elf64` has existed since
earlier in this slice with nothing on the machine using it.

**Decision.** The image carries one program as a Limine module, and the kernel
loads it by walking its program headers.

**The program is assembled by `xtask`, not compiled.** A real userland needs a
second crate, a second link address, and a fourth pass in the gate's
configuration matrix (D-22) — and none of that tests the loader. What the
loader needs is a genuine ELF on a real initrd, and 124 bytes is the smallest
thing that is one. A compiled userland arrives with something worth running.

**The builder is checked against the consumer, not against itself.** `xtask`
writes the header field by field from the specification, and its tests parse
the result back with `libs/elf64` — the crate the kernel uses. A builder
verified by its own understanding of the format agrees with itself whatever it
does; verified against the reader, it proves the two agree. Four host tests,
including that the entry point lands inside a loadable segment, which a
hand-built image gets wrong and a linker never does.

**The refusals are the loader.** An image is bytes a program supplied, so each
of these is a way of asking the kernel to map something it should not:

- **Writable and executable at once** — refused. *W^X is not relaxed for an
  image that asks nicely* (D-12). A loader that dropped the write bit to make a
  program load would be loading something that faults later and blames the
  kernel.
- **Contents that run past the file**, or a segment claiming more file than
  memory — refused, and both are checked with the overflow-safe helpers
  `libs/elf64` provides rather than by the caller remembering.
- **A segment in the kernel's half** — refused against `USER_END` (D-97), never
  by a shift (D-83).
- **Anything that is not an ELF** — refused by the parser, which never panics
  because the bytes are not the kernel's.

**Pages are filled before they are mapped.** `Segment::write_at` puts the image
in the frames while they are nobody's but the loader's, and the mapping that
follows is read-only and executable. The alternative — map writable, copy,
then demote — has an instant where the page is both, and W^X is a property
that holds at every instant or not at all.

**Rationale.** The check that matters doctors a copy of the *real* initrd, one
field at a time, so what is refused differs from what loads by exactly the
thing under test. Breaking the W^X refusal turns it red.

And the loaded program is what the user-copy check now reads back across the
boundary: bytes `xtask` wrote into the image, mapped from a program header, and
read through the *user* mapping by `copy_from_user`. Three subsystems agreeing
about four bytes.

**Consequences.**

- One hundred and one named machine checks.
- `wost::boot` grew a module request and `BootInfo` a `module` field. `enter`'s
  argument list grew past what clippy allows, so the requests became one
  `Requests` struct — which is the shape the handoff should have had anyway,
  since it grows with every protocol feature.
- `Requests` carries no `Debug`, against C-DEBUG, and says why: the bootloader's
  types have none, and a hand-written impl would print the addresses of static
  requests into a log — which is not information anyone wants and is exactly
  what D-46's KASLR ledger would rather not see.
- **Nothing shipped runs a program.** `mod user` is still
  `#[cfg(feature = "selftest")]`: there is an initrd in the image and only the
  checks read it. A shipped caller needs `spawn`, which needs a second address
  space — the rest of this slice.
- The two programs still share the kernel's address space and are loaded at
  addresses shifted apart. That is not isolation and remains the largest thing
  slice 7 owes.

---

## D-101 The third audit: ring 3 could switch SMAP off, and the flush was never executed — Status: Accepted (2026-08-24)

**Context.** A third adversarial pass, over the privilege boundary itself —
`enter_user`'s `iretq` frame, the user copy, the loader parsing initrd bytes,
the TSS padding, and the `VERW` tail. The two before it each found real
undefined behaviour, one of them introduced by the other's repair. This one
found eight defects, and the first is the worst thing any of the three has
turned up.

**Ring 3 could turn SMAP off for the whole kernel.** `EFLAGS.AC` is writable at
CPL 3, and interrupt delivery clears `TF`, `NT`, `RF` and `VM` — **not `AC`**.
So a program that does `pushfq`, sets bit 18, `popfq`, `int 0x80` lands in the
entry path with SMAP already disabled, and it stays disabled for the whole
handler *and for every thread switched to afterwards*, because a context switch
restores no flags. The entry stub had no `clac`. Linux emits `ASM_CLAC` at the
top of every entry point for precisely this.

Nothing in this kernel dereferences a user pointer outside `usercopy`, so
nothing was exploitable — but the mitigation the entire slice is built around
was not armed on the path that matters, and D-97's central claim about a
three-instruction window was false in both directions: the window is bounded by
the copy *plus any interrupt taken inside it*, and it could be opened from ring
3 with no copy at all.

**The program in the image now does it.** Fourteen bytes instead of four: set
`AC`, then call. A hostile initrd, in the gate, on every boot. The check
asserts both halves, because either alone passes for the wrong reason — the
saved flags must show the program really did set `AC`, and the handler must
have been running with it clear. Removing the `clac` turns it red.

**The `VERW` flush was shipped unexecuted, which is the state D-98 was written
to escape.** D-98 put it in the entry stub's tail, reached on a *return* to
ring 3. But the kernel-to-ring-3 transition is `leave_for_user`'s own `iretq`,
which had none — and the syscall handler retires its caller, so there is no
return to ring 3 at all. D-98's consequence prose claimed the crossing "passes
through it"; it does not. The flush now sits on both transitions, and the
kernel-to-user one is the more important: the kernel has just parsed an initrd,
walked page tables and touched the frame allocator.

**`enter_user` accepted another thread's kernel stack.** `prepare` skips the
`RSP0` write for a slot with no arena stack — the boot thread's — so after a
switch to it, `RSP0` still named whichever arena thread ran last.
`kernel_stack() == 0` passed, and the first trap out of ring 3 would have
written a frame onto a stack another thread was standing on. D-99 defended this
with prose: *"it is also the one thread that cannot be in ring 3"*. Nothing
enforced it, and `enter_user` is a public safe function, which D-76 requires to
hold against every safe caller. It now compares against *this* thread's stack
top, and asserts no lock is held — the rule `prepare` and `continue_on_stack`
both enforce, missing from the one function with the strongest reason for it,
since it never returns.

**The loader did unchecked arithmetic on two attacker-controlled words.**
`segment.address() + shift` and `elf.entry() + shift`. In debug that panics the
kernel on initrd bytes — against D-100's claim that the loader "never panics
because the bytes are not the kernel's", which was true of `libs/elf64` and
false of its caller. In release it wraps past the `NotUserMemory` refusal
entirely. `contents_fit` does this correctly one function away.

**A check could not fail, again.** *a thread crosses into ring three* ended with
a `retired` conjunct that waited for a slot to come back — which a retired
thread never returns — and fell back to a comparison true of the checking
thread whatever happened. Deleting `retire_from_trap`'s switch left it green.
It counts retirements now, and breaking the count turns it red. That is the
third audit in a row to find this failure mode, in a commit claiming every new
check was broken.

**Four documents said false things**, including three that named the mechanism
wrong. D-96 and `011` called it *a trap gate* four times; the code builds an
**interrupt gate at DPL 3**, and `libs/x86-tables` says in as many words that a
trap gate "is deliberately unavailable". `011` still carried D-96's
one-thread-in-ring-3 limitation that D-99 had lifted. `xtask`'s program builder
claimed a file-offset congruence it does not hold — describing the bug it says
it avoided. And `USER_END` was open-coded twice in the checks, in the same
commit whose ADR says *"named once. Nothing computes it."*

**Rationale.** The finding worth generalising is the first one. Every mitigation
in this phase was reasoned about from the kernel's side — is the window short,
is the check before the copy, is the range right — and the one that mattered was
a bit the *program* controls, surviving a transition nobody had asked what it
preserved. The audit found it by reading the manual's list of what interrupt
delivery clears and noticing what was absent.

**Consequences.**

- One hundred and two named machine checks. The new one is driven by a hostile
  program that ships in the image.
- `clac` costs a compare and a branch on every trap entry, because this kernel
  has no instruction patching and `clac` is a `#UD` where SMAP is absent —
  which the degraded boot (D-90) exercises.
- **Two findings are recorded and not fixed.** `paging::translate` returns the
  *leaf* entry's flags and never intersects the parents', so four callers state
  a stronger property than they check; it holds today only because
  `Entry::table` always sets `PRESENT | WRITABLE` and `set` upgrades parents to
  `USER`, and the day a parent is read-only or NX, `admits` will pass and the
  copy will take a terminal fault. And **a ring-3 fault still stops the
  machine** — a `ud2` or an unmapped touch in ring 3 panics the kernel, because
  D-31 was written when a fault was necessarily a kernel bug. That is now the
  largest thing this slice owes, and it is in `011`'s record.

---

## D-102 A fault the program caused kills the program — Status: Accepted (2026-08-24), amends D-31

**Context.** D-31 decided that a fault is terminal and reported, and that was
right when it was written: every fault was a kernel bug, and a kernel that
carries on after one is a kernel lying about its own state. Ring 3 changed the
premise (D-96). A `ud2`, a division by zero, or a touch of a page that is not
there can now be a *program* bug, and D-97 already reasons from exactly the
right principle for the copy path — *the program is the untrusted party*. The
trap path did not. D-101's audit named it the largest thing the slice owed.

**Decision.** A fault whose saved `CS` says ring 3 retires the calling thread
instead of stopping the machine, unless it is a fault that means the machine
itself is in trouble: a double fault means the fault path failed, a machine
check is the hardware reporting its own failure, and an NMI is not a program's
to cause. Those stay terminal whoever was running.

The privilege comes from the selector the CPU pushed, which is the hardware's
account of where the fault happened and cannot be forged by kernel code that
merely resembles a program.

**The program can now fault on purpose.** It reads one word of argument from
its own stack — written across the boundary with `copy_to_user`, which is how a
program will get everything it is ever given — and either calls the kernel or
executes `ud2`. Two behaviours from one image, and the check drives the second.

That the fault is *deliberate* is what makes it a check rather than a hope: an
accidental fault would be indeterminate about which fault, and `#UD` is the one
nothing else in this kernel raises.

**Rationale.** The alternative is what was there: a program can stop the
machine. That is not a security boundary, it is a shared fate, and every phase
after this one puts more untrusted code behind it.

The narrower question — whether to *report* the fault before retiring — is
answered by D-31's own rule: nothing logs from trap context, because the
console lock may be held by the code the trap interrupted. The frame is
recorded into atomics and read by whoever wants it, which is what
`last_breakpoint` and `last_ping` already do.

**Alternatives.** Keep faults terminal and let programs be trusted (rejected:
that is not a boundary); retire on *every* fault including double faults
(rejected: a double fault means the fault path itself failed, and continuing is
a claim the kernel cannot support); deliver the fault to the program as a signal
(rejected for now — signals are completion entries by D-80, and there is no
queue yet; the thread that would receive one does not survive this phase).

**Consequences.**

- One hundred and three named machine checks. Breaking the retire arm restores
  the old behaviour exactly, and the report it prints is the evidence:
  *invalid opcode (vector 6) at 0x400000020006, cs 0x33* — the program's own
  instruction, at ring 3.
- **One half of the condition has no check, and cannot have one.** That a
  *kernel* fault stays terminal is proved by the kernel panicking, which is not
  a green outcome — so removing the ring-3 test leaves every check passing. The
  hazard it guards is a kernel bug swallowed rather than reported, which is
  worse than the panic this replaces, and it is guarded by review rather than by
  the gate.
- A retired thread keeps its slot and its mapped pages for the life of the boot.
  Nothing reclaims one yet, and the faulting program makes that more visible
  rather than more urgent: `spawn`'s counterpart is what will.
- D-31 stands for kernel faults, which is what it was written about.

---

## D-103 A walk reports the permission the CPU will grant — Status: Accepted (2026-08-24)

**Context.** `paging::translate` returned the *leaf* entry's flags. The
architecture does not: `USER` and `WRITABLE` are the conjunction down the whole
walk, and `NO_EXECUTE` is the disjunction, so a leaf that says `USER` beneath a
parent that does not is not user memory. Four callers read the result as if it
were effective — `enter_user`'s two checks and `admits`' two — and each states
that stronger property in its own documentation.

D-101's audit found it and ranked it benign, correctly: it holds today only
because `Entry::table` unconditionally sets `PRESENT | WRITABLE` and never
`NO_EXECUTE`, and because `set` upgrades parent entries to `USER` when a user
leaf needs one. The single narrowable bit is repaired on the way down.

**Decision.** The walk narrows as it goes. `USER` and `WRITABLE` are
intersected at every level; `NO_EXECUTE` is accumulated. The leaf's other bits
— cacheability, the owned bit, `PRESENT` — are its own.

**Rationale.** "It holds because of two unrelated invariants elsewhere" is the
shape of a bug waiting for someone to change one of them. The day a parent is
mapped read-only or non-executable — which a second address space with a
read-only shared region would want — `admits` would pass and `copy_to_user`
would take a terminal fault, which is the exact failure D-97 exists to avoid.

Computing it correctly costs one `and` and one `or` per level of a walk that
already reads every level.

**Consequences.**

- One hundred and four named machine checks. The new one is at the *walk*
  level rather than in `libs/x86-paging`, because the property needs a table
  hierarchy where a parent disagrees with a child, and only a real address
  space has one.
- Breaking the narrowing turns **forty** checks red, not one. That is the
  clearest evidence available that this is load-bearing rather than
  theoretical: every guard-page check, every mapping check and every ring-3
  check reads a permission through this walk.
- `Flags::intersection` joins `libs/x86-paging`, host-tested — an operation
  that returned the union, or either operand, would make a walk report a
  permission the CPU does not grant.

---

## D-104 A second address space shares the kernel by reference and nothing else — Status: Accepted (2026-08-24)

**Context.** Everything left in slice 7 — `spawn`, `UFrame` (D-44), real
isolation — waits on a second set of page tables. D-86 made a space mappable
while live and D-87 made every mapping carry evidence; what has not existed is
a *second* space to map into.

**Decision.** `AddressSpace::for_user` makes a space whose lower half is empty
and whose higher half is the kernel's, copied entry for entry from the top half
of the installed top-level table.

**Shared by reference, not by contents.** The entries are copied; the tables
beneath them are not. Both spaces walk the same second-level tables, so the
entry path, the thread stacks, the direct map and the heap are reachable from
either without being mapped twice — and a kernel mapping made later appears in
both, because both are looking at the same table.

That last part has a boundary worth naming: it holds only for a mapping that
lands in a top-level slot **that already exists**. One needing a new slot — a
new five-hundred-and-twelve-gigabyte region — would be invisible to spaces made
before it. Nothing does that today, because every window this kernel uses is
placed before the first space is made, and this is the sentence to read when
that stops being true.

**The empty lower half is the isolation.** Two programs' spaces will share the
kernel and nothing else. Not a claim about mechanism: a walk in one space finds
nothing at an address the other has a page at, which is the check.

**Rationale.** The alternative — mapping the kernel into each space
independently — is what a kernel does when it wants the halves to diverge, and
this one does not: there is one kernel and every space needs all of it. Copying
512 entries once per space is cheaper than any alternative and cannot drift,
because there is nothing to keep in step.

Sharing entries also means a user leaf can never make the kernel's half
reachable. `set` upgrades a parent to `USER` only when a user leaf beneath it
needs one, and user leaves are in the lower half; the shared entries stay
supervisor. D-103's narrowing makes that structural rather than incidental —
even a `USER` parent would grant nothing over a supervisor leaf.

**Consequences.**

- One hundred and five named machine checks. The sharing is checked by
  *physical identity*, not by both walks succeeding: two spaces that each
  mapped the kernel separately would both resolve and would not be sharing.
  And the emptiness is checked at an address the live space *does* map — the
  running program's code page — so the difference between the halves is what
  the check sees rather than the absence of everything.
- Breaking it in either direction goes red: sharing nothing, and sharing all
  512 entries.
- `AddressSpace::lookup` is how anything is asked of a space before it is
  installed, which is the only way to check one that is not the CPU's.
- **Nothing installs one yet.** Switching `CR3` on a context switch is the next
  step, and it brings questions this entry does not answer: which space a
  thread belongs to, what `MappedStack::is_installed` means when the kernel's
  stacks are valid in every space, and a full TLB flush on every switch until
  there is a PCID.

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
- `009-multitasking.md` — Phase 4 charter: the debts D-15, D-34, D-36 and
  D-47 deferred here, the lock order, and the decisions the phase produced
  (D-51..D-66, plus D-76 at its close)
- `011-userspace.md` — Phase 5 charter: the debts D-29, D-44, D-45, D-52, D-56
  and D-69 deferred here, the handle-table and queue designs `002` owed before
  the phase, the lock order that supersedes `009`'s, and the decisions the
  phase still holds open (D-78.. as each is taken)
- `008-research-notes-2.md` — the second research pass, opening Phase 3:
  what the stable toolchain now allows, the evidence behind the decisions
  `007` holds open, and the proposed additions to `005`. Its marks are its
  own; `003`'s tiers still govern what counts as Verified here
- `010-research-notes-3.md` — the third pass, closing Phase 4: verification
  rather than survey. It re-checked `003`'s Tier-2 claims, found no
  independent replication of the Asterinas numbers, answered the framekernel
  SMP question, and produced D-67..D-75. Two of its findings correct earlier
  docs rather than extending them — `002`'s claim about channel delegation was
  false, and `008` §5's Atmosphere proof-to-code ratio of 7.5:1 should be
  3.32:1

The 2026-08-22 adversarial audit closed the gaps recorded in D-39, D-40, and
the amendments to D-22 and D-25. Its two standing lessons, worth reading
before the next phase: a gate is only as wide as its configuration matrix,
and an abstraction that promises a bound must be given something that can
check it.

The 2026-08-23 audit swept phases 0–4 and found fifteen things, none of them
a live bug on either target. Both of the previous audit's lessons had been
broken again, which is why they are recorded as standing: D-64 is two safety
comments asserting checks nothing performed, and D-65 is two configurations
the gate never ran. The rest were corrections — an I/O APIC that subtracted
without checking it owned the line, a counter that counted intent rather than
the transition it was cited for, an `in_trap` assert on one side of the heap
only, a module-wide `allow(dead_code)` broader than its comment, a thread name
nothing read, and three documents describing subsystems that had been
replaced. Its own standing lesson is narrower than the last two and worth the
same weight: **a check's evidence must be the thing it claims, not something
correlated with it.** Three checks failed that test in one afternoon — one
sampled a counter outside the lock it was about, one claimed a reader had not
polled while having no way to tell, and one probed an address an overflow
guard refused before the code under test saw it. Each passed under its own
break until it was fixed.

What the audit did *not* reach, recorded so the next one knows where to start:
xtask's FAT, MBR and GPT writers, `qmp.rs`, `vmware.rs`, the `pixel` packing
arithmetic, and the GDT and IDT bit encodings beyond the TSS.

The 2026-08-23 evening sweep — the one that closed the phase — reached them:
the MBR sizing, QMP token scanning, VMware geometry, `pixel`, `sha256` (NIST
vectors) and the GDT/IDT/APIC encodings were verified against independent
constants and survived; `frame-space`, `eevdf`, `x86-paging` and the ELF gate
each gave up a real defect (the hint-straddling run, the weight past which a
clock stood still, the huge entry whose bit 12 was the PAT, the partially
file-backed `.bss`); and the `wost` soundness pass became D-76. Known and
deliberately unfixed: xtask's QEMU monitor port is probed then rebound (a
race only another local process could exploit), and a repository path
containing a comma would garble the QEMU drive argument — both single-user
tooling risks, neither worth their fix's weight yet.

## D-105 The fourth adversarial audit, over the kernel and Phases 0–6 — Status: Accepted (2026-08-24)

**Context.** D-77, D-94, D-95 and D-101 each attacked a slice. This one was
asked to attack everything: `wost`, `kernel/src`, all nineteen `libs/` crates,
and `xtask` including the gate itself. Four passes ran in parallel, each told
to treat every green check as a claim and to reproduce rather than argue —
the `libs`/`xtask` pass ran sixty mutations and reported twelve survivors.

**Decision.** Fix what was found, and record the two things the fixes changed
about how this repository decides a check is worth having.

*Soundness, in `wost`.* Two ways safe code reached undefined behaviour.
`map_boot` passed caller flags to `map_range` verbatim, and `Flags::OWNED` is
public in `libs/x86-paging` — so safe kernel code could mint a boot mapping
claiming to own a frame it did not, and `unmap`, which consults exactly that
bit before handing frames to the allocator, would give the boot handoff's
memory a second owner. `map_boot`, `map_owned` and `unmap` now strip
`OWNED | HUGE` from what a caller supplies. Separately, `AddressSpace::for_user`
hands out a space whose higher half **is** the live kernel's tables, and every
mapping call wrote through them: `u.unmap(kernel_stack, len)` blanked a running
thread's page table entries and freed its frames. A space now carries a `Reach`,
and one that shares the kernel refuses anything at or above `USER_END`.

*A widening that should have been a refusal.* `set` upgraded an interior table
to `USER` when a user leaf needed one, under a `SAFETY` comment reading "this
space is still under exclusive construction" — a premise D-86 deleted. An
interior entry covers everything below it, so the upgrade handed ring 3 every
mapping already under that table. It is now a `Conflict`, which costs nothing:
a user space's lower half starts empty, so its tables are minted `User` on the
way down. That refusal makes `activate`'s standing claim — "a present entry
accepts only an identical rewrite" — true again, and it is what lets the
effective-permission fold move to `libs/`.

*Evidence that proved the wrong thing.* `BootRegion` meant "the memory map
describes this range", and a `Reserved` device hole is described. A writeback
mapping over one is a cacheable alias of registers. `describe` now requires
`MemoryKind::is_direct_mapped` — D-45's own answer to what counts as memory,
which is the map this evidence exists to build.

*A `#UD` a safe caller could arm.* `interrupt::arm_access_clear` is safe and
public and installs a `clac` in the trap tail; without SMAP that is an invalid
opcode, and the fault handler re-enters the stub that executed it. It asserts
now, like `interrupt::enable` does. A call-site check is what D-77 rejected:
another safe caller can omit it.

*A read that could tear.* D-95 made the TSS stack writes atomic against a trap
arriving mid-write, and the readers undid it on the far side by copying the
whole 104-byte structure. `RSP0` moves on every context switch, so a preempted
reader could see half of one stack and half of another. One aligned eight-byte
load instead.

*A loader with no rollback.* `load_image`'s doc said "every segment is checked
before a page is taken"; the loop checked segment *N*, mapped it, then looked
at *N+1*. Any refusal after the first mapping left earlier segments mapped
`USER` at addresses the image chose, with their frames forgotten into the space
and no owner — while the boot reported "could not load the program". It is two
passes now, with an overlap check validation could not otherwise make, and the
mapping pass walks itself back the way `map_range` does. `LoadError` gained
`Truncated`, `Overlapping` and `TooManySegments` so that one error names one
mechanism — which is what let the `contents_fit` bound be isolated at all.

*Real defects outside the kernel.* `eevdf::Entity::charge` narrowed a widened
product with `as`, so a charge whose virtual value is a multiple of `2^64`
advanced the clock by nothing — the outcome `MAX_WEIGHT`'s own doc says it
exists to prevent, reached by the other door. It clamps now, like its two
siblings always did. `xstate`'s `Layout::compacted` truncated its cursor to
`u32`, silently giving two components one offset, while `is_consistent` —
which the doc calls "the invariant a caller should assert before handing the
size to an allocator" — was a constant `true`. `xtask/src/elf.rs`, the verifier
D-40 says must never panic on a malformed input, panicked on two:
`e_shoff = u64::MAX` and `p_filesz = u64::MAX`. `handle_space::next_generation`
panicked in the debug boot on a raw counter of `u32::MAX`. And the shipped-boot
check stopped consulting the panic flag after its settle window, so a panic
printed in the final poll interval was forwarded to the log and then reported
as a clean boot.

*Two rules, from what the check failures had in common.*

**A check must not be satisfiable by the algebra of its own inputs.** The
`libs` pass found nine of these and gave the reduction for each — a stack
layout proving `base + stride·i ≤ base + stride·i`, an APIC crate whose every
rejection test spelled its input `FIRST_VECTOR - 1` (which panics for every
`FIRST_VECTOR >= 1`), a Pause sequence checked byte-for-byte against itself
(the constant truncated to four bytes passed all thirty-one tests, and the
decoder then emitted two phantom release events), a reservation count asserted
equal to an expression that defines it. The repair is always the same and it is
always cheap: **transcribe the number from the manual**. The crates that
survived the mutation pass — `acpi-tables`, `tss.rs`, `x86-apic/register.rs`,
`x86-pit/command.rs`, `sha256.rs` — are exactly the ones that already did.

**A check must fail for the reason it names, not merely fail.** `load_image`'s
overrun case tripped two refusals at once, so deleting the bound that keeps a
slice index inside attacker-supplied bytes left the gate green. The `VERW`
check read the byte that *arms* the flush rather than the instruction, which is
precisely the split that let D-101 find the flush had shipped unexecuted for a
phase; it now reads a tally incremented beside the `verw`. And
`a_second_thread_crosses_onto_its_own_stack` compared `segment::kernel_stack()`
to itself — global CPU state read twice, long after both crossings — where the
property is the stack each trap actually landed on.

*Where the arithmetic went.* The effective-permission fold left `wost` for
`Flags::through` / `Flags::effective_for` in `libs/x86-paging`. Its machine
check could not fail, and after the `set` refusal above it *cannot*: no
hierarchy this kernel builds has a parent narrower than its child. On the host
a test can hand it one. This is the project's own rule — pure arithmetic goes
where `cargo test` reaches it — arriving at a case where the rule was the fix.

*Four contracts that were written twice and checked nowhere.* The self-test
check count, the verdict version, the four serial markers and the ESP file
names each live in two files. All four were in step, and a drift in any is
found by a sixty-second boot or a forty-five-second image build rather than by
`cargo test`. `paths.rs` and `qemu.rs` now read the sibling source and the
`limine.conf`, and one of them counts `report.check(` calls against
`EXPECTED_CHECKS`.

**Consequences.** Twelve `libs/` crates, four `wost` modules, six `kernel`
modules and `xtask`'s ELF reader changed. Three new machine checks (108 now),
each broken once and watched go red, as were the fold, the stack layout, the
gate encoding, the APIC constants, the Pause sequence, the mmio bound, the
pixel modulus and the charge clamp. `elf64` now refuses an ELF that is not
`EM_X86_64` — an aarch64 image parsed cleanly and yielded segments.

Left owed, and named rather than quietly carried: `xtask/src/elf.rs` still
carries its own transcription of the ELF header offsets rather than using
`libs/elf64`, which is the duplication that crate exists to prevent —
`libs/elf64`'s module doc claimed the merge had happened and now says it has
not. ~~`mm::with_memory_map` and `dev::with_io_apic` are still closures taken
under a leaf guard.~~ **Closed by D-121**: both are gone, replaced by queries
that take the lock, read what was asked, and give it back. ~~And a retired
ring-3 thread still keeps its slot and its pages for the life of the boot.~~
**Closed by D-117**: it becomes a zombie and is collected.

## D-106 A thread's address space is installed by the switch, and the binding is evidence — Status: Accepted (2026-08-24)

**Context.** D-104 built `AddressSpace::for_user`: a space sharing the kernel's
higher half and empty below it. Nothing installed one. Every program ran on
the kernel's own tables at addresses chosen to avoid each other, which slice 5
was explicit about — *"that is not isolation and is not claimed to be"*.

**Options.** A `CR3` write at the crossing, in `enter_user`; a per-process
field the scheduler consults; or the switch itself.

**Decision.** The switch. `task::prepare` is the one place every context change
passes through, and it already writes `RSP0` per thread for the same reason
(D-99). A slot carries a `page_table`, zero meaning the kernel's, and `prepare`
loads it when it differs from what is in `CR3`.

Installing tables is the most dangerous single instruction in this kernel: a
`CR3` that does not map the code about to execute is a triple fault with no
diagnostic. So the root does not travel as a number. `AddressSpace::bind`
consumes the space, compares **every top-level entry of the higher half**
against the live kernel's, and returns a `Binding` — the evidence type
`task::bind_address_space` demands. One comparison covers the kernel's code,
its stacks, the direct map, the trap tables and the device window, and it is
total where sampling addresses is not.

The binding happens while the new slot is still `Reserved` — after `wost` has a
slot and before anything can schedule it. That window is `spawn_in_space`'s
whole reason to exist: a thread's tables, and whatever its entry point must
find, have to be in place before its first instruction rather than racing it.

**Rationale.** The alternative shapes each put the `CR3` write somewhere that
runs *less* often than a switch, which sounds cheaper and is wrong: a thread
preempted in ring 3 and resumed must come back on its own tables, and only the
switch sees that. Putting it in `prepare` also means the check that it worked
is free — `cross` compares `cpu::page_table_base()` against the root it was
given and refuses rather than writing one program's argument onto another's
stack.

**A third door into the higher half, found by breaking the check.** D-105
closed `map_boot` and `unmap` against a space that shares the kernel.
`map_device` was not closed, and the device window is in the higher half: a
user space could walk into the kernel's own device tables and edit them. It
takes the same refusal now. This was found because the break-test for `bind`'s
comparison compiled clean — nothing checked it — and writing the check that
would have caught it turned up the door.

**Consequences.** `load_image`, `place_stack` and `undo` take the space they
map into rather than reaching for the running one, so a program is loaded into
tables no other thread walks. Two checks that read a program's pages from a
kernel thread had to stop: since this decision no program is in the running
space. They map their own probe page instead, which is the honest arrangement
— what they test is the copy and the walk, not the loader.

The isolation claim is now the strong one, and checked: two spaces, one virtual
address, two frames, and the same kernel in both. `two_spaces_map_one_address_to_different_memory`
is in the gate at 110 checks.

Left owed. An `AddressSpace` still has no `Drop` — the deliberate path is
`reclaim`, which D-117 added and the zombie collector calls, so a *retired
program's* space is given back. A space built and **abandoned** without that
call still costs its table frames, and so does a half-loaded program's memory.
Bounded today (three programs and a handful of refusals per boot) and named
here rather than carried quietly; D-44's accounting is where it is settled.
~~Nothing yet unmaps a retired thread's space~~ — **closed by D-117** — and
`APART` still separates two programs' addresses where it no longer needs to.

## D-107 The untyped split is a type, and "untyped" means externally modifiable — Status: Accepted (2026-08-24)

**Context.** D-44 adopted the untyped/typed split as a *type discipline* and
rejected it as an accounting mechanism. `wost/src/mm/mod.rs` has said since
Phase 3 that the split was "absent on purpose … it arrives with the first
caller that can map a frame into a second address space". D-106 built that
caller.

**Options.** Track it beside the frame in the address space; carry a flag on
`Segment`; or make it a separate type.

**Decision.** A separate type, `mm::USegment`, and two doors rather than one
flag. `map_owned` **refuses** `Flags::USER` outright; `map_user` takes a
`USegment` and adds `USER` itself. Converting a `Segment` into a `USegment` is
the caller noticing that it is giving the contents away, and it is the only way
to reach the user door.

The road back is `USegment::reclaim`, which **zeroes first** and is the only
road: a frame that held a program's stack, handed to the kernel still holding
it, is how one process reads another's memory. The zeroing is the conversion
rather than a courtesy attached to it — D-12's zero-on-allocate, applied at the
other end.

**Rationale for the meaning.** "Untyped" here is *externally modifiable, plain
bytes only*. It is deliberately **not** seL4's reading — memory donated to the
kernel to be retyped into an object — because this kernel does not build
objects out of donated memory, and D-44 already refused that. Two systems using
one word for two ideas is how a discipline becomes a vocabulary, so the word is
pinned in the crate doc where the type is.

A flag on `Segment` would have been smaller and would have carried none of
this: a flag can be forgotten at a call site, and the whole value of the split
is that the compiler asks. Adding `USER` inside `map_user` rather than
accepting it from the caller closes the matching hole in the other direction.

**Consequences.** `Segment` gained `read_at`, the counterpart to `write_at`,
and deliberately only on `Segment`: reading a `USegment` into the kernel is
what `reclaim` exists to make impossible without clearing first. The loader,
the stack placer and two checks now go through `map_user`.

`kernel_memory_cannot_be_published_to_ring_three` is in the gate at 111 checks,
and it was broken twice. The second break is the one worth recording: removing
the zeroing from `reclaim` left the first version of the check green, because
it asked whether the frame was still *reachable* rather than what was *in* it.
That is D-105's second rule — a check must fail for the reason it names — and
it took writing the break to see it. `read_at` exists because fixing it needed
a way to look.

Left owed: per-process quota accounting for these frames, which is D-44's other
half and belongs with the handle table's.

## D-108 Extended state is saved eagerly, by the same code that moves `CR3` and `RSP0` — Status: Accepted (2026-08-24)

**Context.** D-82 decided the shape — eager, compacted, boot-sized, out of
line, arithmetic in `libs/` — and D-29 recorded the debt. `libs/x86-cpuid`'s
`xstate` module has been host-tested since slice 5, including the APX case that
made Linux delete its monotonicity assertions. Nothing saved anything. D-99
turned that from a gap into a bug: `RSP0` follows the thread, so two threads
can be in ring 3, so two threads can have floating-point state to lose.

**Decision.** `wost::arch::x86_64::xstate`, and the save sits in
`task::prepare` beside the `RSP0` write and the `CR3` install. Three things
that move with a thread, one place that moves them, for the reason D-99 gave:
it is the only place every switch passes through.

The save happens *before* the switch, not inside it. What makes that sound is
`+soft-float`: between the `xsavec` and the `switch` the kernel executes only
integer code, so there is nothing left in a vector register to lose. That build
flag was already load-bearing for trap entry; this is the second thing resting
on it, and the CI check that it stays is worth more than it was.

`XCR0` is x87, SSE and AVX (D-82). The buffer is `.bss`, one `CEILING`-byte
area per slot, 64-byte aligned — in `.bss` rather than on the heap because the
switch must not allocate and must not fail: a thread cannot be told "no room to
save your registers". The *size* is `CPUID`'s and is checked against the
ceiling at boot, so the array is room and not layout, which is what D-82 meant
by out of line.

**`XSAVEC` where the processor has it, `XSAVE` otherwise, `XRSTOR` for both.**
`XRSTOR` reads the header's `XCOMP_BV` to tell the forms apart, so one restore
serves either and there is no second code path to keep right. D-82 preferred
`XSAVES`; that needs `IA32_XSS` and supervisor state this kernel has none of,
and the component offsets `libs/x86-cpuid` computes are what CET will want, not
what saving needs. The size taken is the larger of the compacted layout's and
`CPUID.0DH.0:EBX`, so one ceiling covers whichever form turns out to be there.
This machine reports 832 bytes and `XCR0` 0x7.

**What the first attempt got wrong, because the check caught it.** Setting
`CR4.OSXSAVE` and writing `XCR0` is not enough to *execute* an SSE instruction:
`CR4.OSFXSR` gates that, and without it every one is a `#UD`. The kernel had
never executed one — `+soft-float` again — so nothing had ever asked. The probe
the check needed was the first, and it faulted on its first boot.
`enable_extended_state` now sets `OSFXSR` and `OSXMMEXCPT` and clears `CR0.EM`
as well.

**Consequences.** `xstate::write_vector_probe` / `read_vector_probe` are the
only things in this kernel that touch a vector register, and they exist so
`a_vector_register_survives_a_switch` can ask. Two threads write different
values to `xmm0` and each still has its own afterwards, which is only true if
the switch away saved and the switch back restored. Broken twice — the save
removed, and `XCR0` narrowed by a bit — and red both times. 112 checks.

Left owed: `#NM` (vector 7) is still reserved for a future XFD-based allocator
and nothing is built on it (D-82); the ceiling is a constant rather than a
growable allocation, which is the shape a wider `XCR0` would force a decision
about; and CET's shadow-stack pointers, which are the reason the component
offsets in `libs/x86-cpuid` are computed at all, have no user yet.

## D-109 The thread table and the run queue are separate structures — Status: Accepted (2026-08-24)

**Context.** `static THREADS` held one `Thread` per slot carrying both what a
thread *is* — its name and entry point — and everything the policy needs about
it. Phase 4 was right to start there: one array, one lock, no boundary to keep
honest. Phase 5 gives a slot a second reason to change, and `libs/eevdf` is
already domain-agnostic and host-tested, so the seam that would let it be
replaced is worth cutting before more code is written across it.

**Options.** Leave it; split the lock; or split the data.

**Decision.** Split the data, not the lock. `Thread` became `Identity` (name,
entry) and `Scheduled` (state, class, entity, timings), and the ten scheduling
methods moved to `impl RunQueue`, which holds the `Scheduled` array and nothing
else. `Threads` owns both halves under the one lock it always had.

**Rationale.** Splitting the lock would be a change to correctness for no
benefit on one CPU: every decision that changes runnability happens with the
other half settled, so two locks would mean an order to get right and a
deadlock to find. Splitting the data costs nothing at runtime and buys the
property that matters — **`RunQueue` cannot name a thread**, because
`Scheduled` has no field to name it with. Swapping the policy is a change to
that impl and to that struct, and the compiler says so.

Slot lifetime is the one thing that spans the halves, so it got one call:
`Threads::release` clears both. A slot free in one and not the other is a
thread with no name or a name with no thread, and that used to be two
assignments a caller could half-perform.

**Consequences.** No check, and this is the honest version rather than an
omission: the property is *structural*, enforced by `Scheduled` having no name
field, so the way to break it is to add one — a visible edit, not a silent
regression. The 112 machine checks all pass unchanged, which is what a refactor
with no behaviour change should look like.

Left owed: `STACK_SLOTS = 16` is still a compile-time default, and the arena is
still mapped at boot rather than per thread. D-86 and D-88 made the second one
possible; nothing yet does it.

## D-110 Channels carry bytes and handles, and both ends declare the rights — Status: Accepted (2026-08-24)

**Context.** `002` names `Channel` as the IPC primitive and pillar 1 rests on
it: *a process receives exactly one handle at spawn — its bootstrap channel —
and every later handle arrived through a channel.* Slice 6 built the handle
table and the object header; nothing could move a handle between tables.

**Decision — the object.** Two ends, made together by `object::channel()`, each
holding a **weak** reference to the other. A send appends to the *peer's*
inbox; a receive takes from its own. Weak rather than strong because two ends
holding each other strongly is a cycle `Arc` will not collect, and the price of
weak is that an orphaned end must say so — `PeerClosed` rather than a message a
caller believes was delivered. Messages already queued survive the far end's
death, because closing a peer does not reach into this end's inbox.

The inbox is bounded at 16 and a full one returns `Full`. Backpressure, not a
promise: nothing is silently dropped, which is what io_uring's `CQ_NODROP`
exists to avoid and the same answer D-79 gave the completion queue.

**Decision — the rights.** Handles travel as `(object, rights)`, never as table
values: a handle value means nothing outside the table that minted it (D-78).
The discipline lives at the table boundary, in two calls that are deliberately
symmetric.

`HandleTable::send(handle, declared)` is a **move**. Three refusals, each a
separate claim:

- **`TRANSFER` is required**, so stripping that one bit yields a handle its
  holder may use and may not pass on. That is the cheapest confinement this
  model has, and it costs a bit.
- **`declared` must be a subset of what is held** — attenuation only narrows.
- **`declared` must mean something for the object type**, which is D-78's rule
  checked where it is easiest to violate.

The refusal is at the **sender**. A receiver policing it would need to know
what the sender held, and the message carries an object and a set of rights,
not a history.

`HandleTable::accept(object, offered, declared)` is the other half, and it is
why **both ends declare** (managarm's `PushDescriptor`/`PullDescriptor`): the
receiver states what it is willing to hold and gets the narrower of the two, so
neither end can be surprised by the other. A receiver asking for more than was
offered is **refused, not clamped** — a clamp is a default by another name, and
Fuchsia RFC-0028's finding is that defaults here have unclear behaviour that is
inconsistent across object types. There is no `send_with_all_rights`.

**Consequences.** Six checks, at 118: the crossing and the bound, the orphaned
end, the two-sided attenuation, the `TRANSFER` refusal, the object identity
across the boundary, and the blocking receive. Four were broken and went red —
`TRANSFER` dropped from the send, the close removed so a send became a copy,
`accept` clamping instead of refusing, and the wait's re-read removed.

**The fourth break is the one worth recording.** Replacing `wait_unless` with a
plain `wait` inside `receive_blocking` left the gate *green*, because
`receive_blocking` takes the message before it ever waits — so it cannot reach
the one state where the two differ. This is D-105's second rule again, and the
fix is the same shape the event's check took: `wait_for_message` drives the wait
directly with the message already queued, so a registration that does not
re-read blocks on a channel nobody will send to again. Written on a spawned
thread, so the failure is a timeout in the observer and not a hang.

Left owed: nothing yet gives a process its bootstrap channel — that is the
handle-at-spawn half of slice 7, and it needs a `Process` to hand it to. The
per-message allocation is charged to no quota (D-44). And a channel end has no
`Drop` that wakes its peer's waiters, so an end dropped while a thread is
blocked on the other side is only noticed at the next wake; the check that
would catch it is the one owed with reclamation.

## D-111 A program receives exactly one handle at birth, and it is a channel — Status: Accepted (2026-08-24)

**Context.** `002`'s pillar 1 was amended once already, because its original
wording — "channels are the sole delegation mechanism" — was false even of
Zircon, which also passes handles through `zx_process_start`. The replacement
was chosen to be checkable: *a process receives exactly one handle at spawn —
its bootstrap channel — and every later handle arrived through a channel.*
Until D-110 there were no channels to give one.

**Decision.** `user::spawn` builds a channel pair before the thread exists,
puts the child's end in a fresh `HandleTable` as its only entry, and returns
the parent's end. The table is made and filled inside the `Reserved` window
`spawn_in_space` opens (D-106), so it is never observable holding anything
else — not "we only ever insert once", but "there is no moment at which a
second entry could be seen".

The parent keeps the far end and nothing else. That is what no ambient
authority looks like from the spawning side: a parent that dropped its end
could never speak to what it spawned again, and there is no name it could look
the child up by.

**Rationale.** The count is the load-bearing half. "It has a bootstrap channel"
would hold with a second handle granted beside it, and a second grant is
exactly the regression this invariant exists to prevent — it is one line, it
looks harmless, and it quietly turns a capability system into an ambient one.
`bootstrap_of` reports the table's live count so the check can assert `== 1`,
and adding an event alongside the channel turns it red.

Quota is 64 handles per process (D-78), which is the answer to kernel-memory
exhaustion at the handle layer. Nothing yet gives a program a second handle, so
the number is room rather than a measurement.

**Consequences.** `spawn` returns a `Spawned` — the thread slot and the
parent's channel end — rather than a bare slot. The check spawns its own
program rather than reusing a launcher, because every ring-3 check counts
crossings and an extra one from here would be attributed to whichever check
looked next; that cost two red checks before it was noticed, and the fix is
that a check which spawns owns its program. 119 checks.

`002`'s Zircon correction was already made when the pillar was reworded; what
this decision adds is the code that makes the sentence true of *this* kernel
rather than a claim about someone else's.

Left owed: the program cannot yet *use* its bootstrap channel — that needs a
syscall surface, which is slice 9's ring. Nothing reclaims a bootstrap when a
program retires, so a table and its channel end live for the boot; that is the
same reclamation debt D-106 recorded and it now has a second holder.

## D-112 The queue protocol is checked before it is written, and each tier is asked only what it can answer — Status: Accepted (2026-08-24)

**Context.** D-84 adopted two verification tiers for `libs/ring-proto`
specifically, on the grounds that the queue ABI is the interface this kernel
will live with longest and the one place where writing code before checking the
design wastes the most work. The crate had unit tests and nothing else.

**Decision.** Three tiers, and the third is new: **the model comes before the
protocol**, and each tier is asked only the question it can answer.

*TLA+, for the whole protocol.* `models/Ring.tla` states D-79's admission rule
and four properties — no dropped completion, reservations are exactly the
unreported operations, never more completions than operations, nothing lost in
between. TLC enumerates all 1040 reachable states of four actors, two of them a
userspace given **no fairness at all**: it may stop reaping for ever, and every
property must still hold. `tla2tools.jar` is vendored and digest-pinned (D-25),
and `cargo xtask model-check` runs it.

*proptest, for the index arithmetic.* Four properties over sequences no
hand-written test would reach, including counters started across the `u32` wrap
with `adopt`. Both are run under `cargo test`, so the gate covers them.

*shuttle, for the interleavings.* Two sides advancing one shared counter, which
is the shape a submission tail and a completion head will have.

**Not in `ci`, for the model check only.** It needs a JVM — the one tool this
project cannot vendor into a Rust build — and a gate step that fails on a
machine without Java is a gate that teaches people to skip it. It is a named
task the phase runs, the treatment `vmware` already gets.

**Three things found by breaking the checks, which is the point of D-28.**

*TLC found an error in my own invariant.* `TypeOK` said `inflight` was bounded
by the submission queue. It is not: operations taken from the SQ stay in flight
while more are submitted, so what limits them is the *reservation* each holds,
and TLC produced the eleven-step trace reaching five in flight with a queue of
four. The model's first job turned out to be correcting the person writing it.

*Shuttle does not model memory ordering.* The first must-fail test was a model
whose only fault was `Relaxed` where `Release`/`Acquire` belonged, and it passed
ten thousand schedules — because shuttle explores thread *schedules* under
sequential consistency and never reorders a store. Memory ordering belongs to
the model and to the reviewer; interleaving is shuttle's. Asking the wrong tool
and believing the answer is the failure mode, and it was one command away.

*And shuttle only sees its own primitives.* The corrected model, built on
`std::sync::atomic`, also passed — because the scheduler has no preemption
points around an atomic it did not instrument, so every schedule was the same
schedule. `shuttle::sync::atomic` finds the split read-modify-write in
milliseconds. Two rounds of a passing wrong version, in one sitting, on the
tier whose whole purpose is to catch them.

**Consequences.** The must-fail tests stay, and the module doc now records what
each tier cannot see rather than only what it can. Breaking the model's
admission rule produces a counterexample naming `NoDroppedCompletion` — the
invariants are ordered in the config so a failure is reported by the name that
says what went wrong. `verify-vendor` iterates every listing rather than the
first, because a vendored directory added and not added there is one nothing
checks.

`getrandom` is pinned to `=0.3.1`: proptest's default features reach it and
0.3.2+ needs a `windows-sys` this toolchain's `dlltool.exe` cannot link — the
same wall `fatfs` avoids by dropping chrono. Recorded as the toolchain tax
`010` §9 measured rather than left to be rediscovered.

Left owed: the kernel has no ring yet. What is settled is the protocol it will
implement and the arithmetic it will implement it with, which is the order this
decision is about.

## D-113 The ring's pages are the kernel's, and every read of them happens once — Status: Accepted (2026-08-24)

**Context.** D-79 settled the queue's shape, D-112 checked the protocol against
every reachable state before this code existed, and `libs/ring-proto` holds the
index arithmetic under proptest. What none of those can be is the part that
touches pages ring 3 can rewrite between two instructions.

**Decision — the pages.** Kernel-allocated, kernel-**kept**. `map_shared` maps
a `USegment` without writing the `OWNED` bit, so the space can never hand those
frames to the allocator, and the ring holds the run for its own life. A program
that unmapped its ring could not free the memory the kernel is about to write a
completion into. That is D-79's "kernel-allocated ring pages, single-owner
rings" made structural rather than documented, and it reuses the ownership
discipline D-88 already built: not-owned means not the space's to give back.

The obligation `map_shared` cannot enforce is stated on it: the run must outlive
the mapping, an `AddressSpace` has no `Drop` and a `USegment` does, and the one
caller keeps the run in the process's `Bootstrap`. Naming what a signature does
not enforce is the alternative to pretending it does.

**Decision — the reads.** Every read of the shared pages goes through a
`Snapshot` taken once per `serve` and never refreshed, and no function takes an
index it did not compute itself. CVE-2025-39816 is literally "always use
`READ_ONCE()` to read ring pointers"; a double fetch is how a validated value
becomes a different value before it is used. `Submission` is a *copy* rather
than a view, so nothing can be re-read after validation — the type enforces it
instead of a comment asking for it.

Three things follow, and each has a check:

- **A submission rewritten after the kernel took it changes nothing.** The
  completion already posted is what the copy said.
- **Admitting is not publishing.** A program may reserve room for three and
  write one; the kernel serves one. The reservation says where a completion
  *can* go, the tail says what is actually there, and serving the difference
  would report completions for entries still holding whatever the page held.
- **The kernel reaps only up to the head userspace published.** Reclaiming a
  slot the program has not read would be the kernel deciding what a program has
  seen.

**Decision — the completions.** `perform` has no path that reaches its end
without a completion, including for an opcode this kernel does not serve — that
is refused *with* a completion rather than ignored. D-79's terminal-completion
rule as control flow. The opcode surface is two, and no privileged ones:
io_uring's exploit record is a warning about surface growth, and Google's
kernelCTF disables it outright.

Every spawned program gets a ring at a fixed address in its own space, which is
what having a space of its own buys — a fixed ABI address costs nothing when no
two programs share a table.

**Consequences.** Five checks, at 124, each broken and red: the shared pages
made ownable, the tail trusted rather than bounded, an unserved opcode
reporting nothing, admission not reserving, and the reap unbounded. Two of the
five needed a second attempt before they could fail at all — the tail bound was
redundant with the kernel's own ring until the check asked about *publishing*
rather than about a runaway value, and the reap bound could not be told from
"reap everything" until the check reaped only half.

`translate_in_root` is new and public: since D-106 no program is in the running
space, so nothing could ask what a program's own tables say. Not behind a
feature, because `wost` has none and adding one would add a pass to the gate's
matrix (D-22) for a function that reads.

**And the stacks grew.** Building a process moves an `AddressSpace` — eight
hundred bytes of device table — through several frames, and a debug build
elides none of the copies. Sixteen kilobytes stopped being enough, and the
overflow arrived as a double fault on a **guard page** in the debug boot D-65
added for exactly this class. That is the whole arrangement working: an
unguarded stack would have written into its neighbour and been found much
later, by something else (D-52, D-89). Thirty-two kilobytes now.

Left owed: the program cannot yet *drive* its ring — the 22-byte hand-written
initrd would need to write an entry and bump a tail, which is a bigger program
than the loader's checks want. The kernel side is exercised through the shared
page, which is the boundary that matters, and a program crossing it is what
slice 10's libc arrives to make ordinary. Nothing blocks on a ring yet, so the
wait queue is built and unused. And no completion carries a handle.

## D-114 The POSIX layer is written and not compiled, and the finding is what it was for — Status: Accepted (2026-08-24), amends D-74

**Context.** D-74 asked for an mlibc sysdep "immediately after the ABI
stabilises, not to run real software, but to find out whether a POSIX libc can
sit on this ABI while the ABI is still cheap to change". Slice 10's plan was to
vendor a portable clang and lld under D-25 and build against them, and it named
its own fallback: if the vendored clang cannot build mlibc after a bounded
attempt, author the sysdeps and the ABI surface, **record the wall with its
cost**, and close without a compiled libc — an ADR amending D-74, not a silent
omission.

**The wall, measured rather than estimated.** This machine has no C compiler of
any kind. LLVM 19.1.7's Windows release is 845 MB compressed and about 3 GB
extracted, and — the objection that actually decides it — it is the **MSVC**
build. LLVM ships no mingw-based Windows release, so a vendored clang would
depend on an MSVC installation this machine does not have and the project has
no way to pin, which is the opposite of the property vendoring exists to buy
(D-16). mlibc additionally needs meson and ninja, neither present, and a target
triple that does not exist and would mean patching its build files.

**Decision.** Take the fallback, and make it a deliverable rather than a
shrug. `userland/abi/wizard_abi.h` states the native ABI as a C header;
`userland/sysdeps/wizard/generic.cpp` implements mlibc's sixteen mandatory tags
against it, each marked with which of three things it is. The counts are the
finding: **3 MAPS, 12 OWED, 1 ABSENT** — since revised to **4 MAPS, 11 OWED,
1 ABSENT** by D-118, which paid `sys_tcb_set`. The current counts are in
`userland/README.md` and are checked against the sysdeps' own tags at every
commit (D-124); the numbers in this paragraph are the finding as it stood on
the day, and are left as they were.

The OWED all have a kernel-side mechanism and no ring-3 door, which is a
much better position than it sounds: `sys_close` over the handle table's close,
the reads and writes over the ring, the memory calls over `map_user` and
`unmap`, and the futexes over `WaitQueue::wait_unless` — which already solves
the lost-wakeup window a futex gets wrong first (D-95).

The one ABSENT is `sys_open`, and it is absent for the reason the design
intends: there is no namespace to name a path in. A libc can be built where
`open` resolves through a directory handle the process was given, which is
`openat` with no `AT_FDCWD` and is what WASI's `path_open` settled on. What such
a libc cannot do is `open("/etc/foo")` from a process nobody gave a root
directory to.

**Four things the exercise surfaced that the native ABI would never have
raised**, which is precisely the value D-74 predicted:

1. **`FS_BASE` is a fourth thing that must follow a thread.** `sys_tcb_set`
   writes it and the kernel does not save it across a switch. `RSP0` (D-99),
   `CR3` (D-106) and the extended state (D-108) all live in `task::prepare`;
   this belongs there with them, and nothing native has thread-local storage so
   nothing native would have asked.
2. **There is no wall clock.** Uptime is monotonic and is all there is; this
   kernel has never been told what time it is. `time()` and `stat` timestamps
   need an epoch.
3. **A seek is state the object holds, not the handle.** Two POSIX descriptors
   to one file share a position — a distinction the kernel will have to make
   about a file object, and by D-78's rule therefore a legitimate per-type
   right.
4. **W^X is a real incompatibility and the right one.** `PROT_WRITE|PROT_EXEC`
   is refused (D-12, D-100), so a JIT maps twice. Named so a port that fails on
   it fails for a stated reason.

**The two questions D-74 said were cheaper now, answered.** A signal is a
completion entry (D-80), so `sigaction` runs in userspace off the libc's own
ring loop — and the consequence writing this made concrete is a constraint on
the *whole* libc, not on signals: **every blocking entry point must be a ring
submission plus a wait**, because a thread blocked in a synchronous call cannot
be interrupted at all. Userland is static-only for now; nothing in the ABI
forecloses a dynamic linker, since the file-backed half of `sys_vm_map` is
OWED rather than ABSENT.

**Consequences.** The header is the ABI written a second time, for a consumer
that cannot read Rust — which is the duplication that made `x86-tables` and
`trap-cause` disagree about vector 28. So it gets the same answer: a host test
compares the ring address, the version, both opcodes, both statuses and the
gate vector against the kernel's own sources, and drift from either side turns
it red. Both directions were broken and checked.

**What is owed, with its routes.** A compiled hello-world. Building LLVM from
source against the mingw toolchain already present is hours and a large
artefact; cross-compiling elsewhere and vendoring the result breaks the
one-machine property D-16 is about; waiting for a mingw-based LLVM release
costs nothing but is not a plan. The sysdeps and the analysis are the part that
answers D-74's question; the compile is the part that confirms it.

## D-115 The Phase 5 closing audit — Status: Accepted (2026-08-24)

**Context.** Slice 11's instruction (D-77's pattern) is to re-attack the
interfaces the phase froze: can a hostile ring-3 program reach UB through any
`wost` export, can a safe caller manufacture a handle, can a ring entry be
double-fetched. The audit was held to D-105's two rules — a check must not be
satisfiable by the algebra of its own inputs, and a check must fail for the
reason it names.

**Decision.** Fix what it found, and record two things it found *about* the
audit process itself.

### Five that were reachable

**A boot check read numbers ring 3 owns.** `ring_shape_of` read the ring's
*header page*, which is mapped `USER|WRITABLE` into a program that is live by
the time the check runs. One store of `0x8000_0000` to `[RING + 4]` makes
`2 * submissions` overflow and panic the debug kernel — and independently, the
whole conjunct asserted a property of userspace-writable memory. `Queue::shape`
now reports the kernel's own record; `published_shape` reads the page and is
documented as an assertion about *publication* and never about shape.

**`xstate::init` enabled SSE and then refused.** `cpu::enable_extended_state`
clears `CR0.EM` and sets `CR4.OSFXSR`, so SSE becomes legal at CPL 3 — and it
was called before three `return None` paths. On any of them `FORM` stays zero,
so the switch saves nothing, and two ring-3 processes share the x87/SSE
register file. Before `init` ran that would have been a `#UD`, so the failure
path left the machine *worse than not calling it at all*. `CPUID.0DH` needs no
control bit, so every refusal now happens first and nothing below can fail.

**`translate_in_root` took a bare physical address.** A safe public function
that walks caller-chosen physical memory through the direct map — whose own
doc says reaching is the caller's to establish — is either a fault on an
unhandled address or an arbitrary-read oracle. This is D-77's named debt in
read form, in the phase meant to discharge it. It takes a `Root` now, mintable
only by `task::address_space_of`.

**`map_shared` and `map_user` could alias one run.** Share it, map it owned,
unmap the owned mapping: the frames go back to the allocator while a live
`USER` mapping still reaches them. `USegment::share` is one-way to a `Shared`
that `map_user` will not take, so the aliasing is no longer expressible.

**Three `lock_leaf` obligations were breached**, all added by this phase.
`Channel::send` reserved a `VecDeque` under a leaf guard — which reaches the
allocator, and `heap`'s own `assert!(!in_trap())` would have turned a
`Channel::send` from a handler into a panic rather than an `OutOfMemory`. Fixed
before the audit reported it, by reserving `DEPTH` once at construction so
`push_back` cannot grow. `Queue::install` held the run's guard across
`map_shared`, which allocates page-table frames: `SpinLock::get_mut` is new for
this, because `&mut self` proves nobody holds the guard and copying the address
out instead would have handed on a number where the type was the evidence. And
`post` took `shared` then `state` where `take` takes them in the other order —
no deadlock on one CPU, and an inversion pre-seeded for SMP.

### Two structural repairs

`map_stack` verified its guard *after* mapping the stack, so a `Conflict` there
returned an error with the stack mapped and its frames owned by the space with
no handle — while `map_owned`'s own doc says "on failure nothing was mapped".
It rolls back now, the same shape D-105 gave `load_image`. And
`USegment::reclaim` **leaks rather than frees** when the clear fails: those
frames still hold a program's bytes, and returning them to the allocator was
the one path where the type's whole purpose lapsed silently.

`serve`'s silent `break` on a missing reservation would wedge a ring for a
process's life — `outstanding()` never drains, `admit` returns zero for ever.
Unreachable by D-112's model, and a `debug_assert` now rather than a bug report
nobody can diagnose. `spawn_prepared` retires `wost`'s slot as well as the
kernel's when binding fails, because releasing only one leaves the two tables
disagreeing and every later spawn that picks that slot refused for ever.

### Nine checks that could not fail, and three that failed wrongly

The reductions are in the audit and each is now either rewritten or deleted.
Three are worth stating because they are the same mistake in three costumes:

- `a_rewritten_submission_cannot_change_what_was_served` read the completion
  slot 128 bytes away from the submission the "hostile rewrite" touched, so it
  reduced to `posted == posted` — **no line in `queue.rs` could falsify it.**
  It now reads the rewritten submission back, so the rewrite is shown to have
  landed, and asserts that serving again yields nothing.
- `xstate::extended_control()` began `if !is_enabled() { return 0 }`, so the
  check's `extended_control() == 0` conjunct read a Rust early return rather
  than `XGETBV`. This is D-105's own `VERW`-arming finding, in a fresh
  instance, three ADRs later. It guards on `CR4.OSXSAVE` now.
- **Nothing in 124 checks reached `posted > SUBMISSIONS`** — the bound that
  stops the kernel walking submission slots nothing was written into. The
  runaway-tail case was tested against a *drained* ring, where `take` refuses
  anyway. The new case admits three and publishes a tail of four billion, and
  deleting the bound turns it red.

`attenuate` returning its own argument meant three "narrowing" assertions were
`X == X`; the genuine content was always in the refusals, and the vacuous
conjuncts are gone.

### Two findings about the audit, not about the code

**One finding did not reproduce as described, and saying so is the point.** The
audit rated `reap_up_to`'s unvalidated head as defeating D-79's headline claim.
It does not: `Ring::reap` returns `None` on an empty completion queue, so a
lying head reaps what is there and no more. I added the explicit bound anyway
and then established, by breaking each in turn, that **neither bound can be
shown load-bearing, because the other covers every input.** Both are kept — the
explicit one states the intent and would matter if `reap` ever became
infallible — and this paragraph exists because a redundant guard with a
confident comment is exactly what the next audit would flag.

**A doc comment claimed an exception did not exist.** `Segment::read_at` said
it was "deliberately on `Segment` alone" because a `USegment` holds ring 3's
bytes — while `USegment::read_at` sat forty lines below forwarding to it. The
sentence was true when written and false within the hour. It now names the
exception and why `Shared` has one.

**Consequences.** 124 checks, and the rewrites mean several of them can fail
for the first time. `SpinLock::get_mut`, `task::retire`, `mm::Shared`,
`task::Root` and `Queue::published_shape` are new. What held is worth recording
too: no double fetch anywhere in `queue.rs`, no userspace index reaching
memory, `serve` bounded whatever ring 3 publishes, handles unforgeable and
unwidenable, and `bind`'s comparison of the whole higher half doing exactly
what D-106 claimed.

Left owed and named: the cross-process arena-occupancy oracle (`Mixer::for_table`
is a public pure function of a small counter, so slot occupancy leaks across
tables — no authority crosses, and the crate doc is honest that the mixer only
removes the accidental case); three unreachable paths in `queue.rs` that
consume a submission without posting its completion, now loud in debug; and the
two `lock_leaf` closure holes D-105 recorded — ~~still open~~, **closed by
D-121**.

## D-116 A program drives its own ring, and the shipped kernel runs one — Status: Accepted (2026-08-24)

**Context.** Phase 5 closed with the async ABI complete on the kernel's side
and unreachable from ring 3. The gate at DPL 3 retired its caller
unconditionally, so a program could cross exactly once and never act on what it
learned; the shared pages, the admission, the terminal completion were all
exercised only by kernel checks. `011`'s close named it the largest thing owed,
and `002` pillar 2 — the native data-path ABI is submission/completion queues —
was a claim about code nothing could use.

**Decision — the gate dispatches.** `rax` selects, and **only `exit` does not
return**. Two calls, and the surface stays small on purpose: io_uring's exploit
record is a warning about surface growth. An unknown number is refused *with an
answer* (`-ENOSYS`), never ignored — a program that gets nothing back cannot
tell a call it made wrong from one the kernel dropped, and would wait for ever
on the difference.

Returning a value needs a writable trap frame, and `TrapFrame` is a read-only
snapshot on purpose. So there is exactly one writer: `interrupt::answer` sets
`rax` and nothing else, and refuses unless the frame is the innermost live one
— the same refusal `switch_from_trap` makes, for the same reason. A stale
snapshot's `origin` names a stack another thread is now standing on.

**Decision — admission happens at the doorbell.** This was wrong first, and the
way it was wrong is worth keeping. The kernel reserved completion slots through
a separate `admit` call, and `serve` took whatever had been reserved. A program
doing exactly what the ABI describes — write an entry, move the tail, ring the
doorbell — was served *nothing*, because the kernel's own ring never learned
about entries it had not admitted. The checks passed throughout: every one of
them called `admit` first, because they were written against the kernel's
shape rather than a program's.

`serve` now reserves as it goes. The reservation is still taken **before** the
operation is accepted, which is the whole of D-79 and what D-112's model
checks: a submission with nowhere to report is left unserved rather than served
and dropped. That is the backpressure, and it is why `serve` can stop early
with entries still published. `reserve_for_test` is what the admission checks
use to exercise it without a program in the way, and it is named so that using
it and publishing a tail — which would double-count — is visibly not the ABI.

**Decision — the program is emitted, not transcribed.** Driving a ring takes
about a hundred bytes of instructions with three conditional branches, and a
hand-assembled listing gets a displacement wrong silently: the branch lands in
the middle of an instruction and the program *runs*, doing something else. So
`xtask` emits it and patches the jumps from labels, a build-time `expect`
catches a displacement too far, and a host test pins the encoding — the ring's
address against `user::RING`, the two `int 0x80`s in order, and every branch
landing on the `ud2`.

The program checks its own answer: exactly one served, the value echoed back,
a clean status. Every failure is a `ud2`. So the program and the check disagree
in the same direction, and a wrong answer is a fault rather than a quiet pass.

**Decision — the shipped kernel runs it.** The userspace modules were gated to
the selftest feature; the syscall path reaches them unconditionally now, and
gating the *caller* instead would have been the wrong repair. **A kernel that
has userspace and never enters it has not shipped userspace** — the loader, the
address space, the handle table, the ring and the gate were dead code in the
image people actually boot. `boot-check` now covers the whole path.

What is genuinely not shipped yet is marked `#[allow(dead_code)]` on the impl
rather than the module, each with the owed syscall named, so a genuinely dead
item still shows.

**Consequences.** 125 checks. `a_program_drives_its_own_ring` transcribes
nothing: it reads back the submission the *program* wrote and compares the
completion against **that**, so a check agreeing with a constant it also chose
is not expressible. The kernel never posts to a program's ring, so a matching
pair can only have come from a program that ran. The tally it counts is
kernel-side, because the program's own is in a page the program can write
(D-115).

Broken twice and red both times: the doorbell answering zero, and the echo
returning the wrong word — the second caught by the program *and* the kernel
independently, which is what the doubled check buys.

The check-count cross-check D-105 added earned its place immediately: bumping
`EXPECTED_CHECKS` without `EXPECTED_SELFTEST_CHECKS` turned `cargo test` red in
a second, where before it would have cost a sixty-second boot to find.

Left owed: the ring carries no handle, so it is not yet the delegation path
channels are; nothing blocks on a completion, so the wait queue is still built
and unused; and the OWED sysdeps of D-114 are still owed (eleven, after D-118
paid `sys_tcb_set`) — this decision
adds the two calls a program needed to prove the ABI works, not the ones a libc
needs to be one.

## D-117 A dead program's memory comes back, collected by whoever has nothing better to do — Status: Accepted (2026-08-24)

**Context.** Phase 5 closed with nothing reclaimed. A retired thread kept its
slot, its stack, its address space, its handle table and its ring for the life
of the boot — three programs a boot made that bounded and uninteresting, and it
stops being either the moment something spawns in a loop, which is what an
operating system does. `011`'s close recorded it; this pays it.

**Decision — who collects.** Not the dying thread. It is standing on the kernel
stack it would have to free and running on the tables it would have to reclaim;
any design where it tries is a use-after-free wearing a comment. So a retired
program's resources become a **zombie**, and two other contexts collect: the
**idle thread**, which *is* "the machine has nothing better to do" and is
therefore exactly the right time to free memory, and **spawn**, because a
machine busy enough never to idle would otherwise never collect — and the thing
most likely to need a slot is the thing most likely to free one.

One per pass rather than all of them. A loop that frees sixteen processes with
interrupts on can be preempted halfway through anyway; one at a time makes that
unremarkable instead of something to reason about.

**Decision — what a space owns.** `take_table` forgot every page-table frame,
which is what makes a table reachable through the direct map with no handle
naming it. That was right and the conclusion drawn from it was not: the
kernel's own tables never come back, but a *user* space's should. An
`AddressSpace` now records its own tables — not the shared kernel entries
`for_user` copies, which belong to the kernel — and `reclaim` gives them back.
Past a ceiling of thirty-two the frames are still correct and simply not
returned, **counted rather than silent**, so the leak is a number somebody can
read.

`reclaim` refuses if these are the tables in `CR3`. Freeing the tables under
the register that is walking them is a triple fault at the next memory access,
and it is exactly the mistake a reclaim path makes.

`bind` therefore **borrows** rather than consumes, with a one-shot flag so a
space still binds at most once. The obligation that replaces the old one is
stated and not enforced: the space must outlive every thread bound to it, which
the caller arranges by keeping it beside the thread's other resources and
freeing them together.

**Decision — the order.** Mappings, then the handle table, then the ring, then
the tables. Each step is why the next is possible: the frames go back before
the tables that reach them do; the table's `Drop` releases the bootstrap
channel, so the parent's end learns its peer is gone; the ring's pages are
*not* among the mappings, because `map_shared` wrote no owned bit and `unmap`
would refuse them — they go back when the ring drops.

**Two tables had to agree.** `retire_from_trap` freed the kernel's slot and
`switch_from_trap` left `wost`'s `Alive`, so every later spawn that picked that
index was refused for ever. `task::retire_to` is `switch_from_trap` with the
leaving slot marked **dead**, which is what makes both the slot and its stack
reusable. This is the same disagreement D-115 found on the binding path, on the
path that actually runs.

**An exit record, because a collected process cannot be asked anything.**
`LastRites` is taken at the moment of death — handles held, what the bootstrap
named, completions served, the submission and the completion. Before this,
every check that inspected a retired program was reading a corpse the kernel
had not got round to burying, and those checks broke the instant burial
started. Keeping the record is also just right: "what did that program do" is
worth answering once the program cannot answer it.

**A check's premise changed, and the check had to say something else.**
`a_second_thread_crosses_onto_its_own_stack` asserted that two crossings landed
on *different* stacks. That held because a retired slot was never reused — so
two crossings could not share a stack even if `RSP0` never moved, which makes
it a weaker claim than it looked. Slots are reused now and two threads that
never coexist correctly share one. The property is that a trap lands on the
arena slot belonging to **the thread that made it**, so the entry path records
the calling slot and the check checks that. Strictly stronger, and reuse-proof.

Three other checks compared free-slot counts for equality, which became a race
the moment user threads began freeing their own slots asynchronously. They wait
for `>=` now: the property is that *this* thread's slot came back, and another
coming back with it does not falsify it.

**Consequences.** 126 checks. `a_dead_program_gives_its_memory_back` measures
in **frames**, not slots: a freed slot says a thread ended, which was already
true — the question is whether the memory came back, and the allocator is the
only thing that can answer it. It then runs **forty programs through sixteen
slots** and requires the frame count not to fall, which without reclamation
fails at the seventeenth spawn and would have leaked hundreds of frames before
that. Broken twice and red both times: the unmap removed, and the table
tracking removed.

Left owed: a program's *stack* is still the kernel's boot-time arena rather
than mapped per thread, so `STACK_SLOTS = 16` remains a real cap on
concurrency even though it is no longer a cap on programs ever run. Nothing
reclaims a space with more than thirty-two of its own tables. And a zombie
waits for a collector, so a machine that neither idles nor spawns holds them —
bounded by the slot count, which is the honest version of "eventually".

## D-118 Thread-local storage is the fourth thing that follows a thread — Status: Accepted (2026-08-24)

**Context.** D-114 wrote mlibc's sixteen mandatory sysdeps against the native
ABI and named four things the exercise surfaced that nothing native would have
raised. The first was that `sys_tcb_set` writes `FS_BASE` and the kernel does
not save it across a context switch — a hole in a place three neighbouring
decisions had already visited, and one only a POSIX consumer would have looked
for, because nothing native has thread-local storage.

**Decision.** `FS_BASE` joins the privilege stack (D-99), the tables (D-106)
and the extended state (D-108) in `task::prepare` — the one place every switch
passes through, which is now the whole answer to "what follows a thread". Two
MSR accesses on a path that already writes two control registers.

Saved from the *register* rather than from a remembered value, because ring 3
can change it: a program that set its own would otherwise have the kernel
restore the one it was given.

**The safe door range-checks, and against the user boundary rather than
canonicality.** `wrmsr` to `FS_BASE` with a non-canonical value is a general
protection fault *in kernel context* — a program crashing the handler rather
than itself — so `set_thread_base` refuses first and writes second. It refuses
the kernel's half as well, which canonicality alone would allow: a program
asking to point `fs:` at kernel memory has no reason it could be given, and on
this architecture the two checks are one because the boundary is where the
canonical hole begins.

`paging::is_user_address` is that check, and it is a **range comparison, never
`>> 47 == 0`** (D-84): the shift form encodes an assumption about how many bits
the machine walks, which five-level paging changes.

**Consequences.** `WIZ_CALL_TCB_SET` is the third implemented call, and the ABI
header's numbering moved with it — the owed calls renumbered rather than
leaving a gap, because a gap is a number somebody eventually fills wrongly.
`sys_tcb_set` moves from OWED to MAPS: four map now, eleven are owed, one stays
absent.

127 checks. `thread_local_storage_follows_the_thread` sets a base, has a second
thread set its own, and requires each to keep what it set — which is only true
if the switch saves and restores. Broken twice and red both times: the restore
removed, and the range check removed.

Left owed: `GS_BASE` and `swapgs`, which are still not paid and still have no
caller that would benefit on one CPU — the phase close said Phase 12 with SMP,
and this decision does not change that. A thread's base is not inherited by
anything, because nothing yet creates a thread inside an existing process.

## D-119 Breaking the checks is a task, not a habit — Status: Accepted (2026-08-24)

**Context.** D-28 is the rule this project takes most seriously: every new check
is broken once and watched go red, because a check that cannot fail is worth
less than no check — it is also a claim. Five adversarial audits have now found
roughly twenty checks that could not fail, several of them in the same commit
as an ADR asserting they had been broken.

The rule is not wrong. **Doing it by hand is.** A break-test happens once, at
the moment the check is written, and never again — so a check that *stops*
being able to fail, because the code around it changed, is invisible from then
on. That is not a discipline problem; it is a problem no amount of discipline
fixes, which is what makes it worth automating.

**Decision.** `cargo xtask mutate`. Each entry is a real defect — a bound
removed, a flag not stripped, a guard inverted — and applying one and booting
must turn some check **red**. A mutation that survives is a property nothing
checks, and the report names it.

Ten to begin with, chosen to be *load-bearing rather than merely different*:
each removes something something else depends on, and none is a cosmetic edit
that would fail the build instead. A mutation that cannot compile has not been
tested by anything, so a build failure is reported as **inconclusive and
counted as a survivor** — reporting it as caught would be the same lie this
task exists to find.

**Two tiers, and the cheap one matters more.** `cargo test` requires every
mutation's pattern to occur **exactly once** in its file — no boot, no build,
milliseconds, and therefore in the gate. A pattern that has stopped matching is
a mutation that silently stopped testing anything, which is precisely the
failure mode this file exists to catch, one level up. Zero matches and two
matches are both errors: with two, which one it breaks is not decided.

The expensive tier is one QEMU boot per mutation and is **not in `ci`**, for
the reason `model-check` and `vmware` are not: a gate step that costs ten
minutes is a gate people learn to skip.

**Restoring is not best-effort.** The original is held in memory, written back
*before* anything is allowed to fail — including the check on whether the
mutation was caught — so no error path can leave the tree modified.

**The first run found a defect in itself, which is the same class of finding
and exactly as welcome.** The mutation for "a boot mapping cannot launder a
frame into the allocator" targeted the flag strip in `map_run`, which adds
`OWNED` back on the next line — so stripping the caller's changed nothing and
the mutation survived. The property was checked all along; the *mutation* was
not testing it. A hand-written break-test makes that mistake exactly as easily
and never reports it.

Retargeted at `map_boot`'s own strip, all ten are caught.

**Consequences.** The next audit starts from a shorter list: these ten
properties are now checked *and demonstrated to be checkable*, continuously,
rather than at one moment in the past. Adding a mutation is the natural
companion to adding a check, and the gate will notice when one rots.

Left owed: ten mutations is a sample, not a sweep. The obvious growth is one
mutation per invariant an ADR claims — which would make "this decision is
enforced" a thing the machine says rather than a thing a document asserts. And
nothing yet mutates the `libs/` crates, where `cargo test` is fast enough that
a full sweep is seconds rather than minutes.

## D-120 A fault in the user copy resumes, because the program is the untrusted party — Status: Accepted (2026-08-24)

**Context.** D-97 built the user-copy ABI from the right premise — *the program
is the untrusted party* — and did not act on it. Every page of a range is
walked before a byte moves, so a bad pointer is refused rather than faulted on;
but the walk and the copy are two moments and **interrupts are on between
them**, so a thread preempted mid-copy whose pages are then unmapped faults on
memory that was there when it looked. That fault was terminal, which means a
program could stop the machine by arranging it. `011`'s close named it the last
correctness hole in the boundary.

**Decision.** An exception table, in the shape every kernel that has solved
this uses. The copy is hand-written assembly with the two instructions that can
fault between named labels, and a fixup label after them. A page fault whose
instruction pointer falls in that range resumes at the fixup, which returns how
many bytes were left; the caller turns that into `UserCopyError::Vanished`.

**Assembly rather than a loop**, because a compiler-generated
`copy_nonoverlapping` has no stable instruction range — nothing could say "a
fault *here* is recoverable" about one. Byte at a time, which is slower and is
the price: the ranges this kernel copies are a few bytes to a page, correctness
at the boundary is worth more than the difference, and a wider copy would need
one fixup per width.

**The redirect is `pub(super)` with exactly one caller.** "Set the instruction
pointer" is the primitive every exploit wants, and a general one is not
something this kernel should have. `usercopy::recover` is the only caller and
it has already established that the fault was inside its own two-instruction
range and that the target is its own fixup label. The narrowness is the whole
safety argument.

**The recovery is consulted *before* the ring-3 test.** This fault is taken in
ring 0 — the kernel is the one dereferencing — so `is_the_programs` says no and
the machine would stop. Getting the order wrong would leave the mechanism built
and unreachable, which is D-101's finding about the `VERW` in a new place.

**What it buys beyond the hole it closes.** Once a fault in the copy is
recoverable, dereferencing an arbitrary *user* address is memory-safe: the
worst outcome is a short copy and a number saying so. `copy_from_user_unwalked`
is that — range-checked so it can never touch the kernel's half, but not
walked. It is what Linux's `copy_from_user` has always been, and the walked
forms are kept because they give a **better answer**: a program that passed an
unmapped pointer learns that, rather than learning that some bytes did not move.

**The check had to be driven through the unwalked form**, and the reason is the
rule this project keeps re-learning. `admits` refuses an unmapped address
before a byte moves, so a check built on the walked copy tests the refusal it
already has and the recovery *not at all* — it would pass with the fixup
deleted. Pointing the unwalked form at nothing is the mechanism itself.

**Consequences.** 128 checks. `a_faulting_user_copy_resumes_and_reports` counts
recoveries rather than inferring one from the machine still running, requires
the kernel's half to be refused before the window opens, and requires a mapped
page to still copy — so a fixup that had simply broken the copy is not a pass.

Broken twice, and **both breaks panic rather than going red**: without the
fixup the fault is terminal and the self-test never reaches its verdict. The
harness treats a panic and a red verdict alike, which is right — a kernel that
died is a kernel that noticed — and `cargo xtask mutate` carries the mutation
with that noted, because a reader would otherwise expect a red check.

`unsafe` in `wost` is 354 lines against a budget of 380.

Left owed: nothing yet unmaps another thread's memory, so the window this
recovers from is not reachable on this machine — the count stays at one, from
the check that drives it deliberately. That is the honest state: the mechanism
is proved and the race it guards is not yet constructible. It becomes
constructible the moment a process has two threads.

## D-121 A lock that forbids calling has no closure form — Status: Accepted (2026-08-24)

**Context.** D-105 split the lock API in two. `lock(&mut ThreadContext)` takes
a context, and the borrow checker then refuses a second acquisition while the
guard lives. `lock_leaf()` takes nothing, which is what makes it usable from a
trap and from `Drop`, and it buys that by moving the obligation onto the
caller: **nothing is called while the guard is live.** A leaf lock is the last
one in the order because nothing under it can reach another.

Three statics offered a closure over their contents — `with_memory_map`,
`with_io_apic`, and the `with_active_space` shape they were copied from. Each
took its leaf guard, handed the caller a reference, and ran the caller's code.
A closure *is* a call. The obligation was on the caller, nothing checked it,
and the audit found a caller walking page tables under `MAP`'s guard: a second
structure, with its own reachable locks, entered with the map's held.

The reference the D-105 sweep had, and did not act on: this is the shape of the
callback-with-lock-held bug that Rust's own `RefCell` guidance names, and the
reason Linux's `->readdir` was reworked into `iterate_shared` — a caller-
supplied function under a lock is an unbounded call, and its bound cannot be
read from the site that takes the lock.

**Options.**

1. **Document the obligation harder.** Free, and exactly what was already
   there. The comment said it and a caller violated it, which is the whole
   argument against.
2. **Rank the leaf locks.** A type-level order under `lock_leaf` would catch a
   *lock* taken under one. It would not catch the page-table walk, which takes
   no lock — it dereferences, and a fault there is taken with the guard held.
3. **Delete the closure forms.** Replace each with queries that take the lock,
   read what was asked, and give it back — a number, an `Option`, or a copy.

**Decision.** Option 3. `with_memory_map` became `map_covers_memory`,
`first_region`, `past_every_region` and `direct_mapped_extents`;
`with_io_apic` became `routing_of`. Where a predicate is genuinely needed the
argument is a **`fn` pointer, not a closure**: it cannot capture, so it cannot
reach a lock the caller happened to be holding, and the whole of what it can do
is readable at its definition.

Where the answer is a structure rather than a number, it is **copied out and
the guard dropped** before the caller touches it. `direct_mapped_extents`
returns sixty-four extents by value — a page and a half of stack, which the one
caller's boot-time walk has room for — and reports the true count separately,
so a map with more stretches than fit makes the caller refuse rather than
silently check part of memory.

**Rationale.** The obligation stops being an obligation. A query takes the lock
and returns; there is no window in which caller code runs with the guard live,
so there is nothing for a caller to get wrong. That is the same move D-105 made
one level up — replacing a rule a reviewer must apply with a shape the compiler
or the API enforces — and it is worth more here, because the failure mode is a
deadlock or a fault-with-guard-held rather than a wrong answer.

The cost is real and small: four calls where there was one, four lock
acquisitions instead of one, at boot, on an uncontended lock.

**Consequences.**

- Three closure-taking accessors are gone. Nothing in the tree hands a caller a
  reference under a leaf guard.
- `MAX_EXTENTS = 64` is a new ceiling, and a caller past it refuses. That is a
  bound where there was an unbounded loop under a lock.
- `first_region` takes `fn(&MemoryRegion) -> bool`. The non-capturing
  restriction is the point, not an inconvenience: a `move` closure over a guard
  is precisely the thing being prevented, and the type refuses it.
- The rule generalises and is written here so the next static does not have to
  rediscover it: **a `lock_leaf` static may not expose a closure form.** If a
  caller needs to do work with the contents, the contents are copied out.

**Left owed.** The rule is not mechanically enforced — a fourth static could
grow a `with_` method tomorrow and nothing would say so. A lint over `wost` and
`kernel` for a `pub fn` taking `impl FnOnce` in a module that also calls
`lock_leaf` would catch it, and is not worth writing until there is a fourth.
Recorded so the next one is a decision rather than an oversight.

## D-122 The syscall handler leaves a number; something else says it — Status: Accepted (2026-08-24)

**Context.** D-116 gave the gate a dispatcher and made the shipped kernel run a
program. The unknown-call arm answered `-ENOSYS` and then logged the refusal.
Logging takes the console's `ThreadLock`, whose first line is
`assert!(!interrupt::in_trap() || is_panicking())` — the assert D-63 put there
so that a lock a handler must not take says so rather than deadlocking.

So **`mov rax, 3; int 0x80` from ring 3 halted the machine.** Unprivileged,
unconditional, no recovery, and shipped — the initrd is untrusted input by
D-100, and D-116 put a program in the boot image people actually run.

It was green because **no check had ever issued an unrecognised call number.**
Every check drove the two calls that exist. The arm had never executed.

It also falsified three comments at once, all written in the same commit:
`user::call`'s "nothing here may allocate, block, **or take a lock a handler
cannot**", `trap.rs`'s "**Nothing here logs**", and D-116's own "refused *with
an answer*, never ignored" — describing a path that halts before the program
can read the answer.

**Decision.** The handler leaves a number; something else says it. The refusal
increments a counter and stores the number asked for, and the **idle thread**
reports it — a kernel thread, in a context where taking the console's lock is
legal. Once per refusal, so a program in a loop cannot turn a refusal into a
flood.

That is the general shape and worth stating as one: **a trap handler that
wants to say something writes a number, and a thread says it.** The handler
cannot know which locks its caller holds; a thread can.

**Consequences.** 129 checks. The program gained a third behaviour — argument
two asks for `0x0BAD_0BAD` and then exits — so the arm is now executed on every
boot, which is the only reason a check about it can fail. `an_unserved_call_is_refused_without_stopping_the_machine`
reads the *count*, because "the machine is still here" is established by the
check running at all.

Broken once, and the break panics at `sync.rs:406` — the assert itself, which
is the defect being a defect. `cargo xtask mutate` carries it.

**And the program's own jump was hand-computed and wrong.** Adding the third
behaviour meant a new `jz`, and computing its displacement by hand put it two
bytes past the target and into the middle of an instruction — where it
assembled, ran, and did something else. That is precisely the failure D-116
built the label-patching machinery to prevent, made by the person who built it,
in the same file. The `jz` is patched from its label now like every other
branch, and the encoding test checks where it lands.

**A second finding, from the same audit, about this session's own process.**
An adversarial audit was mutating `libs/x86-apic` to find survivors when a
`git add -A` here swept its in-flight edit and its `.bak` into a commit.
`BOOTSTRAP` shipped as `1 << 9` with its own doc comment one line above still
saying bit 8, so `main` was red while the working tree was not. `git add -A` in
a tree another process is editing is the mistake, and it is recorded here
because the same reflex will be available next time.

Left owed: the two other audits from this pass have findings not yet acted on —
`send_self_interrupt` accepting a vector the CPU pushes an error code for,
`reclaim` not consulting `bound`, `unmap` handing back user pages unzeroed, and
a list of checks that cannot fail. Each is named in the audit and is next.

## D-123 Four ways safe code reached undefined behaviour, and the road back that was not the only one — Status: Accepted (2026-08-24)

**Context.** Two adversarial audits over Phases 0–5, held to D-105's two rules.
D-122 took the critical finding; these are the rest of the high ones.

### An eight-byte frame shift, through three doors

The entry stubs push a synthetic zero error code exactly where the CPU does
not. `apic::send_self_interrupt` asserted only `vector >= 16`, so
`send_self_interrupt(17)` — a safe call — delivered an interrupt on a vector
the CPU pushes an error code for, into a stub that pushes a second one. The
frame is eight bytes out for the rest of its life: the tail's `add rsp, 16;
iretq` pops `CS` as `RIP`, and the ring-3 test reads the wrong slot.

An APIC delivery pushes no error code whatever the vector *means*, which is
what makes the vector's meaning and the frame's shape disagree.
`set_timer_one_shot` and `interrupt::init`'s `user_gate` are the same door
twice more — the last one ring-3-callable. All three refuse
`exception::pushes_error_code` now; the predicate already existed.

### `reclaim` freed the tables a bound thread would be switched onto

The only guard was "are these tables in `CR3` *right now*", which a caller
reaches by not being the thread in question. `prepare` loads a slot's root
unconditionally, so freeing a bound space's tables is a fault deferred to
whenever that thread is next scheduled — worse than an immediate one for being
harder to attribute.

Whether anything still names a space is a property of the **task table**, which
`AddressSpace` cannot see. So it takes the answer: `task::release_address_space`
looks, clears the binding from the dead slots naming it, and mints a `Released`.
Guessing with a `CR3` comparison was the shape of the bug.

Two shapes changed along the way, and both were the audit's finding one level
down. `reclaim` **borrows** rather than consumes — a refusal that consumed the
thing it declined to act on leaves a caller unable to try again and leaks the
tables it was protecting. And on success the space is *emptied* rather than
dropped: `is_reclaimed` is true and `may_reach` refuses, which is what replaces
consuming it. Returning the space in an `Err` was the first attempt and clippy
was right to refuse a thousand-byte error variant.

`address_space_of` now reports only a **live** slot's tables. A dead slot's are
nobody's; reporting the last root it had is an answer about the past.

### `cpu::enable_extended_state` was a `pub` front door to the state D-115 fixed

It clears `CR0.EM` and sets `CR4.OSFXSR`, so SSE becomes legal at CPL 3. D-115
fixed the ordering *inside* `xstate::init`; any safe caller could reach the
same state from outside, where `FORM` stays zero and nothing saves the register
file across a switch. `pub(super)` — the ordering argument is about one call
site, and a public door made it an argument about nothing.

### `unmap` was a second road back, and reclamation took it

`USegment`'s doc said `reclaim` is the only road out of untyped memory and that
it zeroes. `unmap` checked `OWNED` and never `USER`, so it handed a program's
pages over as a plain `Segment` — and `user::reclaim` takes exactly that road,
dropping its token rather than flushing it. A dead program's pages went back to
the allocator **holding its bytes**.

Nothing disclosed anything, because every consumer happens to zero on the way
in. That is the honest limit and also the point: the mechanism named as the
guarantee was not the one doing the work, and defence in depth was carrying a
property its owner claimed.

`Unmapped` carries `was_user` now — any page ring 3 could reach makes the whole
run untyped, because a run is handed back as one thing. `flush` refuses such a
range, `take_user` gives a `USegment`, and `Drop` clears before the frames go
back.

**The clearing on drop could not be checked, and I said the mutation tool
covered it.** It did not: removing the clear left all 130 checks green and
`cargo xtask mutate` reported the mutation as a **survivor** — which is what the
tool is for, applied to a claim in a comment I had just written. Those frames
go to the allocator, and reading them again needs it to hand back the same
ones, which it does not promise. So the clear is *counted*, and the tally is
what makes the line testable. A second survivor, "a bound space is not
reclaimed", was the same thing: a real property with nothing constructing the
case. Both have checks now, and all fourteen mutations are caught.

**Consequences.** 131 checks. `a_user_page_comes_back_cleared` follows one
handle the whole way — map, unmap, `take_user`, `reclaim`, read — rather than
asking the allocator for the frame back, because a conjunct conditional on
reuse is one skipped on most boots.

Left owed from these audits: the `libs/` mutation survivors (`x86-cpuid`'s four
SDM constants, `handle-space`'s three identity asserts, `acpi-tables`'
`Sdt::LEN`, and five more), `xtask/src/elf.rs`'s missing `e_machine` and
`e_phentsize` refusals, the gate's two enumeration holes, the ABI cross-check's
19 unchecked constants, thirteen checks that cannot fail, and a list of doc
comments. Each is named in the audits and none is a soundness hole.

## D-124 A constant that only equals itself, and a check that only measures its own description — Status: Accepted (2026-08-24)

**Context.** The sixth adversarial sweep over Phases 0–6 went looking for the
same failure D-28 exists to prevent, and found it in two shapes neither the
rule nor `cargo xtask mutate` reaches.

*The first shape: a constant pinned to itself.* Roughly fifteen `libs/`
constants were asserted only against expressions that named them. `assert_eq!(
GENERATION_BITS, 32 - INDEX_BITS - RESERVED_BITS)` is a definition restated;
`assert!(bit & bit != 0)` is true of every single bit; a sweep whose ceiling is
spelled `MAX_WEIGHT` cancels the constant on both sides. Every one was changed
and the suite stayed green. `x86-port-space`'s `PORT_COUNT` was halved and
nothing failed, because x86's I/O space being sixteen bits appeared nowhere but
in the constant. `SPEC_CTRL_MSR` was referenced by no test at all.

*The second shape: a check that measures a description.* The frame allocator's
run check asserted the run's *address* was aligned, its *count* was eight, and
the allocator's *tally* moved by eight — three properties of what the allocator
reported, all of which hold for an allocator that hands back eight scattered
frames and describes the first. The heap's reuse check compared frame counts,
and a slab carves hundreds of slots per frame, so leaking every free never
reached a refill. The clock sweep seeded `previous = 0`, which made the first
comparison vacuous — and attempt zero is the one sample taken under a lock,
which is the whole reason it is special. The console's check was named "scrolls
without reading the device" and read a boolean saying a buffer had been
allocated.

*And a third, found by the fix to the first two.* `D-121` was cited in eight
places across `wost` and `kernel`, and no such entry existed in this log. Every
one of those comments reads as a pointer to a rationale and every one led
nowhere. Nothing in the gate looked at this file.

**Options.**

1. **Fix the instances.** What the previous five sweeps did. It works once, and
   the sixth sweep is evidence about how long it lasts.
2. **Extend `cargo xtask mutate` to `libs/`.** It boots a kernel per mutation,
   which is the wrong instrument for a host test — and a mutated constant is a
   *compile*, not a boot, so the cost is entirely wasted.
3. **Fix the instances, and add the checks that make each class visible.**

**Decision.** Option 3, with the classes handled differently because they fail
differently.

*Constants get a second source.* Each is now asserted against the number from
the document that fixes it — the SDM, the ELF specification, the architecture —
written as a literal at the assertion. `PORT_COUNT` is checked against 65,536,
not against `PORT_COUNT`. A literal beside the constant is a transcription
error away from wrong, which is a real cost and the smaller one: a wrong
transcription fails loudly the first time it is compared, and a self-comparison
never fails at all.

*Checks measure the thing.* A contiguous run is written across its whole span,
a frame taken from underneath it must land outside, and the marks must survive.
The heap's reuse is the **same address back** — the free list is LIFO, so a
freed slot is the next slot of that class, and a leak makes it a different one.
`black_box` on each allocation is load-bearing and was found the hard way:
without it LLVM elided the pair, both addresses were equal because neither
allocation happened, and the check went red with the heap correct. The clock
sweep seeds from a real reading. The console's check is renamed to what it
looks at.

*The log gets a gate.* `xtask/src/decisions.rs`, two host tests: the entries run
from `D-01` upward with no gap and no repeat, and **every `D-NNN` written
anywhere in the tree names an entry that exists**. Milliseconds, in `cargo
test`, so it runs at every commit.

**Rationale.** The two shapes share a root, which is why they are one decision:
a check written from the *implementation* rather than from the *claim* passes
by construction. `GENERATION_BITS == 32 - INDEX_BITS - RESERVED_BITS` is what
the code says; the claim is that a handle is thirty-two bits wide because that
is what fits a register. The run check asserted what `allocate_contiguous`
returned; the claim is that the frames are consecutive. Restating the
implementation always passes, and reads exactly like a check.

D-28's rule — break it once and watch it go red — catches this at the moment
the check is written and never again. Both classes here were written correctly
and *became* unfailable: `MAX_WEIGHT` moved and the sweep followed it;
`allocate_contiguous` grew a tally and the check started reading it. That drift
is what the mutation task was built for and what a task cannot reach when the
mutation is a `const` and the verdict is a compile.

The decision-log gate is the same argument at the level of the record. Two
hundred comments cite this file. The citation is load-bearing — it is how a
future session finds out *why* — and one in eight places pointed at nothing.

**Consequences.**

- Fifteen `libs/` constants now carry the number from the manual beside them,
  with the source named. Changing one fails a host test.
- Six kernel checks measure what their names claim. Each was broken once and
  watched go red, per D-28, and the breaks are in the session record.
- `xtask/src/decisions.rs` is in the gate. It failed on its first run, on the
  citations to this entry written before this entry existed — which is the
  behaviour, not an accident of ordering.
- D-121 is written, reconstructed from its eight citation sites. It says what
  they say it says: a `lock_leaf` static may not expose a closure form.
- A new ADR must now be written before it is cited, or `cargo test` refuses.
  That is a small, welcome friction: it is the same rule as D-19's
  charter-before-code, one level down.

**Left owed.** The citation check proves a number exists, not that the entry
says what the citing comment claims. Nothing can check that but reading, and
the sweep that found this one read all eight. Recorded so a later session does
not mistake a green gate for a verified rationale.

## D-125 Evidence has a date, and a root match is not a re-walk — Status: Accepted (2026-08-24)

**Context.** `MappedStack` is the value `map_stack` returns: a stack top, plus
the root of the space that mapped it. It is not `Clone`, so exactly one caller
holds it, and `segment::set_double_fault_stack` takes it as the proof it needs
before writing the CPU's `IST1` entry — the stack the most privileged handler
on the machine will stand on.

`is_installed` was `is_active() && active_root() == self.root`. That answers
*which space the CPU is walking*, and it was written to answer that: addresses
collide across spaces, so the root is the only thing that tells two stacks
apart. What it does not answer is whether the stack is **still there**.

`MappedStack` is evidence dated at construction. Nothing consumes it on unmap,
nothing invalidates it, and the root goes on matching for the life of the
space. So the value outlives the mapping, and a caller holding one can point
`IST1` at an address the tables no longer translate. A double fault then takes
a second fault on its own stack, which is a triple fault: the machine reboots
with nothing on the wire.

D-94 found this shape once already — `map_owned` taking its own doc at its
word — and left this instance on its open list with the honest note that
nothing in the tree unmaps a stack. That is still true. It is not an argument:
the same sentence was true of the user-copy window until D-120 built the check
that reaches it, and true of `reclaim` until D-123 found four callers that
could.

**Options.**

1. **Consume the `MappedStack` on unmap.** The strongest: the type system would
   make a stale one unconstructible. It needs `unmap` to know which of its
   pages were a stack, which it does not, and would put a stack-shaped special
   case into the general path.
2. **A generation on the space, bumped by every unmap.** Cheap to check and
   coarse: any unmap anywhere invalidates every stack in the space, so the
   double-fault stack would stop being installed because a thread exited.
3. **Re-walk the mapping in `is_installed`.** Ask the tables the question the
   caller is actually asking.

**Decision.** Option 3. `is_installed` keeps the root comparison and adds a
walk of the stack's last byte, requiring the flags `map_stack` established:
present, writable, never executable. `top` itself is *not* walked — it is one
past the region, and belongs to whatever is above it or to nothing.

**Rationale.** The caller's question is "may I point the CPU at this now", and
the tables are the only thing that answers it *now*. A root comparison answers
a question from the past and reads like an answer to the present, which is the
same confusion D-94 named: restating a property in a new place is not the same
as holding it.

The cost is one table walk, at boot, once per stack. Option 1 is better and
this does not foreclose it — it makes the value's answer correct while the
value is still a value, which is what a later linear-typed version would also
need for its `is_installed`.

Why not option 2: a generation makes the answer correct and *useless*. The
double-fault stack is installed once at boot and asked about for the life of
the kernel; the first thread to exit would invalidate it. An answer that is
conservative in the direction of refusing the machine's fault handler is not
conservative.

**Consequences.**

- `is_installed` walks. `set_double_fault_stack` and `set_privilege_stack` are
  its callers and both already treat `false` as a refusal, so nothing else
  changed.
- A new machine check, `an unmapped stack is no longer installed`, maps a stack
  above every window the kernel uses, confirms it is installed, unmaps it
  underneath the value, and requires the answer to flip. Broken once against
  the old root-only body and watched go red (D-28).
- The flags are checked, not just presence. A stack that was remapped
  read-only, or executable, is not a stack either, and the walk is already paid
  for.
- D-94's open item is closed. The narrower reading it left — "nothing in the
  tree unmaps a stack, so the window is not reachable" — is now false by
  construction: the check reaches it deliberately.

**Left owed.** The walk is of one page. A stack whose *middle* was unmapped
would still answer `true`, and `map_stack` maps the whole run at once so
nothing produces that state today. Walking every page would make the answer
exact and costs a walk per page at boot — worth doing if a stack ever grows or
shrinks after it is mapped, and not before.

## D-126 A leaf lock is the last lock, and now something says so — Status: Accepted (2026-08-24)

**Context.** D-121 wrote the rule down: a `lock_leaf` static may not expose a
closure form, because a leaf guard's whole obligation is that **nothing is
called while it is live**. Its "Left owed" said the rule was not mechanically
enforced and that a lint would be worth writing when there was a fourth
offender.

There were four already. Nothing had looked.

The obligation is asymmetric, and the first version of the check got it
backwards. A leaf taken *under* a token-bearing lock is the design — it is what
makes D-47's `HEAP` → `FRAMES` order sound, and it is the only acquisition a
`Drop` can perform, since `drop` takes no arguments and a freed `Frame` must
reach the allocator from wherever it dies. It is the other direction that is
the violation. A check counting "deepest nesting" flagged `Segment::drop`
taking `FRAMES` under a caller's guard, which is exactly what `lock_leaf`
exists to permit.

**What it found, once it asked the right question.**

- **`pic::silence`** held `CONTROLLERS`' guard and claimed ports under it,
  which takes `CLAIMED`'s. An order `CONTROLLERS → CLAIMED` established by
  accident, in a file that never mentions the other lock.
- **`user::serve_ring`** held `BOOTSTRAPS`' guard across the *entire* service
  of a program's ring: reading its memory, performing each operation, posting
  completions, and waking whatever was waiting. Four more locks, all under a
  guard whose contract is that nothing is called at all.
- **`user::ring_shape_of`** and **`user::bootstrap_of`** did the same, smaller:
  `BOOTSTRAPS` → the ring's own lock, and `BOOTSTRAPS` → the handle arena.

**A second defect, found by a peer session auditing the same window.** The
user-copy recovery arm in `trap.rs` had the asymmetry D-124 had just fixed on
the syscall arm. It runs *before* the ring-3 test — it must, because the fault
it exists for is taken in ring 0 with the kernel dereferencing — and it never
looked at the saved `CS`. A ring-3 thread that set `RIP` inside
`[wost_copy_fault_start, wost_copy_fault_end)` had its instruction-fetch fault
"recovered": `resume_at` rewrote `RIP` to the fixup with `CS` still ring 3, it
faulted again there, and only then was retired. Not an escalation and not a
halt, but a program could drive `USER_COPY_RECOVERIES` at will — and the arm's
own comment asserted a privilege nothing enforced.

**Options for the enforcement.**

1. **A lint over the sources**, as D-121 imagined. It would have to decide
   whether a call reaches a lock, which is the whole call graph, statically,
   across two crates.
2. **A `debug_assert` in `acquire`.** Loud, immediate, and only in the debug
   boot — and the violation is a latent deadlock rather than a crash, so a
   panic overstates what happened and stops the boot before the other three
   are found.
3. **A counter and a boot-time check**, reporting where.

**Decision.** Option 3. `LEAVES_HELD` is incremented only by `lock_leaf` and
`lock_from_trap`, and any acquisition made while it is non-zero records
`Location::caller()` for **both ends** — the lock being taken, and the
outermost leaf that was held. A boot-time check reports the pair and fails.

The outermost, specifically. Recording every leaf acquisition names whichever
was taken most recently, held or not, which sent the first reading of this
check to a function three call levels from the real one.

The four violations are fixed by the same move D-121 made: what the caller
needs is copied out and the guard dropped. `Bootstrap`'s ring and handle table
are behind `Arc`s so a clone is what leaves the guard; `pic::silence` claims
its ports between the two acquisitions rather than under either.

The trap arm is gated on `frame.code_selector() & 0b11 == 0`, and a **real
ring-3 thread** drives the check: the emitted user program gained a fourth
behaviour — an argument in the kernel's half is jumped to — and the kernel
spawns one aimed at `usercopy::fault_window()`. Ring 3 cannot execute kernel
memory, so this is an instruction-fetch fault with `RIP` inside the window and
`CS` at ring 3, which is the exact state the arm accepted.

**Rationale.** D-121's rule was written and immediately not followed, four
times, including twice in code written after it. That is the argument for
enforcement over documentation, and it is the same argument D-119 made about
D-28: a rule that is applied once, at the moment the code is written, is a rule
that decays silently.

Why runtime rather than static: the property is *dynamic*. `serve_ring` takes
no lock itself — it calls `Queue::serve`, which calls `WaitQueue::wake_all`,
which takes one. No local reading of `serve_ring` shows a violation, and the
lint that would have to see it is a whole-program call-graph analysis. A
counter sees it on the first boot that exercises the path, which is what a
boot-time check is for.

What this does **not** catch: a path no boot exercises. `handle.rs`'s
`reserve_a_slot` drops the old `Vec` under `ARENA`'s guard — an `ARENA` → `HEAP`
order — and it is latent, because the first growth frees a zero-capacity
vector and a second growth needs sixteen live handles. The check will name it
the day a boot does that, and not before. Stated plainly so a green result is
not read as a proof.

*It named it the same day.* D-128 wrote a check that holds forty handles in one
table, walking the arena's capacity through 16 → 32 → 64, and the detector
printed `heap.rs` under `handle.rs:171` on the first boot that ran it. Two
things worth keeping from that. The scenario was built **before** the fix, so
the check is honest by D-28's standard — it failed once, for the reason it
exists. And the violation had **no call site**: `reserve_a_slot` is correct
about every acquisition it *makes*, and the lock was taken by a destructor at
scope exit. All four violations above are visible calls, findable by reading;
this one is not, and a static lock-order analysis would have to model
destructor insertion across every scope exit to see it. That is a better
argument for the runtime counter than anything in this entry.

**Consequences.**

- Every `SpinLock` acquisition costs one relaxed load and a branch. The counter
  beside it already cost that.
- `acquire` takes a `leaf: bool`, and `lock`/`try_lock` pass `false` while
  `lock_leaf`/`try_lock_leaf`/`lock_from_trap`/`try_lock_from_trap` pass `true`.
  `SpinLockGuard` carries the flag so its release knows which counter to undo.
- `Bootstrap::ring` and `Bootstrap::table` are `Arc`s. Both are read on paths
  that must not hold the table's guard, and both have only `&self` methods.
- Two machine checks: `no lock is taken while a leaf guard is held`, which
  names both ends when it fails, and `a program cannot aim at the copy window`.
  Both were broken and watched go red (D-28); a mutation covers the second.
- The branch-target assertion in `program.rs` was itself wrong and said so: it
  scanned for the byte `0x75` and counted a *displacement* that happened to be
  `0x75` as a branch. It consumes the pair now. A byte scan over emitted code
  is a decoder pretending not to be one, and this is the second time that has
  cost something (D-115's jump displacement was the first).

**Left owed.** ~~The check is one word for the whole machine, which is right on
one CPU and wrong the moment there are two — `LEAVES_HELD` must move into
`arch::percpu` beside `SPINLOCKS_HELD` before an AP starts, or two CPUs will
report each other's nesting.~~ **Closed by D-154**, which built
`arch::percpu` and moved both counters and the violation record behind it.
`arch::percpu` did not exist when this was written, which is why the sentence
says "move into" a module that had to be created first.

`lock_from_trap` counts as a leaf, which is conservative: a trap handler's
acquisition is take-mutate-return by the same argument, but nothing checks that
it is, and the panic path takes locks under it deliberately.

## D-127 A channel end that dies says so — Status: Accepted (2026-08-25)

**Context.** `Channel::receive_blocking` documents two ways to return: with a
message, or `None` when "the far end closed with nothing left to read". The
second was unreachable for the only caller that could need it.

The loop is receive, test `is_orphaned`, wait. `WaitQueue::wait_unless`
registers, re-reads its condition once, and blocks — and a blocked thread
re-reads only when something wakes it. So `is_orphaned` is evaluated on the way
*into* the wait and never again. A receiver that blocks while its peer is
alive, and whose peer then dies, is waiting on a condition that has become true
and on a queue nothing will ever wake. It sleeps for the rest of the boot,
holding its slot.

Nothing woke it because nothing could. `Channel` had no `Drop`. An end is an
`Arc<Object>`; when the last reference goes, the `VecDeque` and the `Weak` are
released and that is all that happens. The peer's `Weak` upgrade starts failing
at that instant, and no code runs to tell anyone.

This is not a shape waiting for a future caller. `user::reclaim` drops a dead
program's handle table in a stated order, and step 2's stated reason is "the
bootstrap channel among them, **so the parent's end learns its peer is gone**".
A parent blocked on that channel learned nothing.

The check that was supposed to cover this — `an orphaned channel end refuses` —
polls `is_orphaned` from the checking thread and never blocks. It is a check
about the *predicate*, not about the wake, and it passes on a kernel with no
wake at all. D-95 found the same distinction on `Event` and stated it: the
state can only be re-read once a block returns.

**Options.**

1. **Poll.** Give a blocked receiver a deadline and let it re-test. Turns a
   tickless wait into a timer, for a condition that changes once.
2. **Signal the peer from every path that can drop an end.** `HandleTable`'s
   `Drop`, `close`, `send`, `reclaim`. Every one of them, for ever, and the
   next path to be written is the one that forgets.
3. **`Drop for Channel`.** The end that dies wakes the other, wherever it dies.

**Decision.** Option 3. `Drop` reaches the peer through `SpinLock::get_mut`
rather than a guard and calls `wake_all` on it.

`get_mut` is load-bearing, not tidiness. A `Drop` runs wherever the last
reference dies — inside `reclaim`, inside a handle table's teardown, at the end
of a `send` that failed — so a lock taken here is a lock taken under whatever
that caller happened to be holding, which is what D-121 forbids and what
D-126's check now reports. A `&mut self` needs no lock, so there is none to
nest.

**Rationale.** Option 2 is where this kind of bug is manufactured: a property
that must be restated at every site is a property that holds until the next
site. D-94 named that pattern and D-126 named it again about D-121 — a rule
applied once, at the moment the code is written, decays silently. `Drop` is the
one place the language guarantees runs on every path, including the ones not
written yet.

Option 1 is worse than it looks. `002` wants a machine whose only outstanding
business is a wait to take no interrupts at all, and a receiver polling for a
peer that will die once is exactly the arrangement D-57 removed.

The recursion terminates at depth two: reaching the peer takes a strong
reference, releasing it can destroy the peer in turn, and that peer's own drop
finds this end's strong count already at zero and upgrades nothing.

**Consequences.**

- `Channel` has a `Drop`, so its fields can no longer be moved out. Nothing
  moves them.
- One machine check, `a blocked receiver learns its peer is gone`: a spawned
  thread blocks in `receive_blocking`, the check waits until it is genuinely
  off the run queue, then drops the last strong reference to the other end.
  **Nothing is ever sent**, so the wake under test is the only thing that can
  end the wait — which is what makes it red rather than slow when the wake is
  absent. Broken and watched go red (D-28): without the `Drop` the thread never
  returns and holds its slot for the rest of the boot.
- `object::channel` refuses rather than returning an unlinked pair. The `if let`
  around the peer linking had no `else`, so a failure would have handed back two
  ends that satisfy the type and violate the invariant — every `send` reporting
  `PeerClosed` for a peer that was never opened, arbitrarily far from the cause.
  Unreachable, and written as a refusal anyway: this kernel's shape is that a
  constructor which cannot establish its invariant returns `None`.

**Left owed.** `Event` has the same shape and does not need it yet: an event is
signalled rather than closed, and nothing waits on one whose last reference can
go while a waiter holds a reference to it. The day an object's death is a
meaningful answer to a wait, it needs this treatment too, and the reasoning
should be here rather than rediscovered.

## D-128 Growing the arena is two allocator operations, and one was left behind — Status: Accepted (2026-08-25)

**Context.** `handle.rs`'s `reserve_a_slot` exists for one reason, and its doc
says so: `Vec` growth calls the allocator, which takes `HEAP` and possibly
`FRAMES`, and doing that under `ARENA`'s guard would rebuild the `ARENA` →
`HEAP` order D-85 deleted. So the new vector is allocated with no lock held and
handed over afterwards.

The handover was `arena.slots = grown`. That assignment drops the vector it
replaces — which after `append` has length zero and still owns its buffer. So
the *free* happened with the guard live, and a free is `dealloc`, and `dealloc`
takes `HEAP`. The function reconstructed the exact order it was written to
prevent, in the one line that was supposed to be the safe half.

Only half the operation had been moved out. Allocating and freeing are two
calls into the allocator, and the reasoning that identified the first as a
hazard did not carry to the second.

It was latent, and stayed latent for a reason worth recording: the first growth
replaces a vector of capacity zero, which owns nothing and whose drop is not a
`dealloc` at all. The second growth is the first one that frees anything, and
it needs sixteen handles live at once. No boot had ever held sixteen. D-126's
check is a runtime counter and sees only paths a boot walks, so it named this
nowhere — and D-126 says so in its own text, naming this function as the
worked example of what it cannot catch.

**Options.**

1. **Copy the entries out and back** rather than moving the vector, so no
   vector is discarded. Trades one allocation for `n` moves and leaves the same
   drop somewhere else.
2. **Keep the old buffer**, growing into a second vector and never releasing the
   first. A leak that grows with the arena.
3. **Name the discarded vector and free it after the guard**, which is what the
   function already does for the allocation.

**Decision.** Option 3. The `if` becomes an expression yielding whichever
vector this call is left holding — the arena's old buffer where the handover
happened, the unused room where another caller got there first — and the guard
is dropped explicitly before it.

The explicit `drop(arena)` is required, not decoration: the discarded vector is
declared after the guard, so end-of-scope order would release it first, which
is the bug.

**Rationale.** Option 1 is the version that looks safest and is not. Moving `n`
entries under the guard replaces a bounded `dealloc` with an unbounded copy, and
the discarded vector still has to be freed somewhere — the drop moves, it does
not disappear.

The general shape is the one D-124 named: a comment that states a property is
not the property. This function's doc asserted twice that every acquisition it
makes is a leaf, and the assertion was read as evidence for eight commits.

**Why it stayed hidden is the part worth keeping**, and it is a different
failure from the four D-126 fixed. Those were all *visible calls* — a function
that takes a lock, called from under a guard, findable by reading the call
site. This one had no call site. `reserve_a_slot` is right about every
acquisition it makes; the lock it took was taken by a `Drop`, and a `Drop` is
invisible at the point that triggers it. There is nothing to read.

That is the strongest argument yet for D-126's counter over a lint. A static
analysis of lock order would have to model destructor insertion across every
scope exit to see this, which is most of a borrow checker for a property the
borrow checker does not have. A runtime counter sees it the first time a boot
executes the drop, and needs to know nothing about why the drop is there.

**Consequences.**

- One machine check, `the handle arena grows more than once`: forty handles in
  one table, which walks the arena's capacity through 16, 32 and 64. Two
  reallocations, so the second one — the first that frees a real buffer — is on
  the path.
- Broken and watched go red (D-28), and the red is D-126's check rather than
  this one's own assertions: on the unfixed code the boot prints
  `wost/src/mm/heap.rs took a lock under the leaf guard at kernel/src/handle.rs`
  and fails. That is the intended division — this check is the *scenario*, and
  D-126's is the detector. It is also the first thing D-126's check found that
  D-126 had not already fixed.
- The arena's growth path is now exercised by every self-test boot, which it
  was not. Forty handles is above `HANDLE_QUOTA`, so no program reaches it;
  before this, only the first growth had ever run.

**Left owed.** `HandleTable`'s own `Drop` sizes its release vector before taking
the guard for the same reason and gets it right, and `close` releases the object
after the guard for the same reason and gets it right. Both were written by
someone holding the rule in mind. The rule is now checkable on any path a boot
walks (D-126), and unwalked paths remain the gap — the arena's third growth is
reached by this check, a fourth is not.

## D-129 Three claims about a boot-once function, and the safe door nobody could use — Status: Accepted (2026-08-25)

**Context.** D-126 reordered `pic::silence` so its port claims happen outside
both leaf guards. A peer session auditing the same window read the comment I
left and found it asserting something the code does not do — which is the
class D-124 exists for, introduced by the commit that was fixing that class.

**The comment said the recheck answers the race. It does not.** Interleave two
callers: B reads `CONTROLLERS` as `None` and drops the guard; A claims both
port blocks; B's two claims fail; B takes `CONTROLLERS`' guard and finds it
**still** `None`, because A is queued behind B on that same guard; B falls
through and reports `PortsUnavailable` for a PIC that is about to be silenced
correctly. The recheck closes the window only once A has *stored*, and nothing
makes it have done.

Two further things the audit surfaced around it:

- **A failed partial claim strands what it got.** `PortBlock` has no `Drop` and
  the registry never releases — right for a driver that owns its registers for
  the life of the kernel, and a trap for a caller claiming a *set*.
  `claim_block(PRIMARY).zip(claim_block(SECONDARY))` claims PRIMARY, fails on
  SECONDARY, and leaves PRIMARY spoken for by nothing, so no later caller can
  ever silence the PIC. The `?`-chained version this replaced had the same
  outcome and merely reached it less reliably.
- **`segment::set_kernel_stack` had no callers, and could not have any.** It is
  the evidence-taking form of the `RSP0` write: it takes a `MappedStack` and
  refuses unless `is_installed()`. The context switch writes `RSP0` *before* it
  installs the arriving thread's tables, and since D-125 `is_installed` walks
  the running tables — so the arriving thread's stack is never in the space the
  CPU is walking at the moment `RSP0` is set. The switch calls
  `write_kernel_stack` instead, and always did.

**Options for the race.** Restructure `silence` to be correct under
interleaving — a claim-then-publish with the loser releasing — or state the
window and record it as owed.

**Decision.** State it, and record it against D-56.

`silence` runs once, at boot, from one caller. The interleaving needs two, and
this kernel has one CPU. Making it race-free buys a code path nothing on this
machine can execute and no check can reach, which is the shape D-115 kept
finding: an unreachable branch is a claim nobody has tested. The hazard arrives
with the second CPU, and so does the other half of the same work — D-126 owes
`LEAVES_HELD` a move into `arch::percpu`, for the same reason and on the same
schedule. Both belong in D-56's per-CPU pass.

What the code carries instead is the interleaving, written out, with the reason
it is not fixed. The flag *is* set between the two guards, so it is a
preemption away rather than SMP-only — named, because "unreachable on one CPU"
and "unreachable" are different sentences.

The partial-claim consequence moves to `port`'s module doc, where it belongs:
it is a property of the registry, not of the PIC. `silence` names it rather
than guarding, because on the one path it runs the ports are free.

**`set_kernel_stack` is deleted**, and `write_kernel_stack` records why. A safe
wrapper that its only intended caller cannot use by construction is the
speculative API `005` forbids, and leaving it in suggested a road that was
never open — a reader finding it would reasonably assume the switch had simply
not been updated to use it.

`mm::managed_frames` goes with it, on the plain version of the same rule:
nothing has ever called it, `frame_space::FrameSpace::managed_count` is reached
through the stats struct where it is actually wanted, and a `pub fn` in `wost`
is part of the kernel's interface to its own TCB.

**Rationale.** The first two are one idea: a comment that overclaims is worse
than no comment, because the next reader spends their scepticism elsewhere. The
peer found this by reading the comment against the code rather than by
suspecting the code — which is the only way this class is ever found, and the
reason D-124 put as many of these claims as it could under a test.

Deleting the dead door is the same idea one level up. `set_kernel_stack` is a
*claim in the shape of a function*: it says the kernel has a checked way to set
`RSP0`. It does not, it cannot, and the reason it cannot is interesting enough
to write down — D-125 made `is_installed` honest, and an honest `is_installed`
is exactly what makes this function unusable. A safety improvement closed a
door that had looked open.

**Consequences.**

- `pic::silence` is unchanged in behaviour. Its comment now describes the
  interleaving it does not survive, the permanence of a partial claim, and why
  neither is fixed here.
- `port`'s module doc states that a caller wanting two claims must survive
  getting one. There is one such caller.
- `wost` loses two `pub` items and 155 becomes 153. Neither had a caller;
  neither is replaced.
- No check is added, and that is a decision rather than an omission. The race
  needs a second CPU; the partial-claim path needs a port the PIC does not
  already own; the deleted function had no callers to regress. What the gate
  *does* now hold is that `wost`'s unsafe count is pinned (D-124), so removing
  code cannot quietly change it — `MEASURED` is unaffected because neither
  deletion contained any.

**Left owed.** The two-caller race, and `LEAVES_HELD`'s move into
`arch::percpu`, both against D-56. Whoever does that pass should treat this
entry as its inbox: they are the same work, and neither is checkable before the
second CPU exists.

## D-130 An `allow` is unfalsifiable and an `expect` is not — Status: Accepted (2026-08-25)

**Context.** `kernel/` carried twenty-six `#[allow(dead_code)]`. Each one is an
assertion — *this item has no caller in the shipped build, and that is
deliberate* — and each was written when it was true. Nothing checked that any
of them still was.

Seven had stopped being true, and every one of them stopped for the same
reason: the shipped kernel grew a caller. D-116 made it run a program, so
`user::spawn` became live, and with it `HandleTable`'s working half, `Entry`'s
fields, `HandleError`'s variants, `ObjectType`, `Bootstrap`, `spawn_in_space`
and `spawn_weighted`. The attributes stayed. So did the prose beside them:
`spawn_in_space` said "no shipped caller: the shipped kernel reads the keyboard
and runs no user program", and `HandleTable`'s working half said the shipped
kernel reached none of it. Both were describing the kernel of two phases ago,
in a doc comment a reader would have no reason to doubt.

This is D-124's shape — a comment that states a property is not the property —
with an attribute standing in for the comment. It is worse than a comment in
one respect: a comment is at least visibly prose, and `#[allow]` looks like
enforcement.

Three of them had a second problem. `spawn_weighted`, `spawn_in_space` and
`yield_now` carried the attribute **between the halves of their doc comment** —
legal, since a doc comment is itself an attribute, and it renders as two
paragraphs with nothing to say they were ever one. Nobody reading the built
documentation would see anything wrong.

**Options.**

1. **Audit them by hand, now.** Fixes today's seven and nothing about
   tomorrow's.
2. **Delete the attributes and `cfg`-gate the items instead.** Correct, and it
   means the shipped image stops carrying the code the checks exercise — so a
   check would be testing a different binary from the one that ships, which
   D-22 already refuses on the linting side.
3. **`#[cfg_attr(not(feature = "selftest"), expect(dead_code))]`.** State it as
   an expectation rather than a permission, and gate it on the configuration
   the claim is actually about.

**Decision.** Option 3, for the nineteen that are still true; the seven that
are not are deleted along with the prose that repeated them.

`expect` fails the build when the lint it names does *not* fire. So the
attribute is now falsifiable from both sides, and the gate already builds both
kernel configurations with `-D warnings` (D-22):

- if a shipped caller appears, `unfulfilled_lint_expectations` fires in the
  build without the checks;
- if the item goes dead in the checked build too, `dead_code` fires there,
  where there is no attribute at all.

**Rationale.** This is D-28's argument moved off checks and onto attributes. A
check that cannot fail is worth less than no check, because it is also a claim;
an `allow` that has stopped being needed is exactly that, and it is silent for
ever. Twenty-six unfalsifiable assertions is not a tidiness problem, it is
twenty-six things the gate was believed to be holding and was not.

Which seven were stale was found **empirically rather than by reading call
sites**: the attributes were removed wholesale, both configurations compiled,
and the compiler named them. That is the same move as breaking a check to watch
it go red — the answer comes from the machine, and reasoning about call graphs
across a `#[cfg]` boundary is exactly the reasoning that produced the seven.

One property had to be confirmed rather than assumed, because the whole
approach rests on it: rustc's dead-code pass seeds its worklist from items
whose lint level is `Allow`, and `Expect` is a different level. If `expect` did
not seed the same roots, converting would have reported every item an allowed
one transitively used. It does seed them — the conversion produced seven
unfulfilled expectations and not one new `dead_code` — so the two forms differ
only in what happens when the claim stops being true, which is the whole point.

**Consequences.**

- Nineteen `#[cfg_attr(not(feature = "selftest"), expect(dead_code))]` in
  `kernel/`, each of which the gate now holds us to; seven attributes and three
  stale sentences gone.
- Broken and watched go red (D-28): putting the attribute back on `ObjectType`,
  which has had a shipped caller since D-116, fails `cargo xtask clippy` with
  `this lint expectation is unfulfilled`. That is the failure mode the whole
  change exists to buy, so it is the one that had to be seen.
- The three attributes wedged inside a doc comment are out of it. `yield_now`
  keeps its expectation and now carries it immediately above the item, where an
  attribute belongs.
- No check count changes and no behaviour changes. The image is byte-identical;
  what changed is what the gate refuses.

**Left owed.** `wost/` has its own `#[allow]`s and has not been swept. The same
treatment applies and the same empirical method finds them, but `wost` has no
feature to gate on — its dead code is dead in every configuration — so the form
there is a plain `#[expect(dead_code)]` rather than a `cfg_attr`, and the
sweep should be its own pass rather than folded into this one.

`#[allow(unused)]` and the other blanket allows are not covered here. Every one
of them is the same kind of claim and none of them is checked.

**Amended by D-131 and D-132.** The conclusion above — that converting `allow`
to `expect` makes an unfalsifiable claim falsifiable — has two limits, both
measured after this was written and neither foreseen in it. An `expect` inside a
**macro body** is reported by nothing, which makes it *worse* than the `allow`
it replaces rather than better: it reads as checked. And an `expect` asserts
that a lint does fire, so it is wrong wherever the suppression is a standing
exemption rather than a specific construct — `wost`'s crate-level
`#![allow(unsafe_code)]` says `wost` is the only crate that **may** hold unsafe,
not that it does. D-131 has both, with the measurements. Nothing above is
retracted; the sweep it describes was over `kernel/`, where neither limit
applies.

## D-131 Where `expect` cannot reach, and the difference between a permission and a claim — Status: Accepted (2026-08-25)

**Context.** D-130 replaced `#[allow(dead_code)]` with
`#[cfg_attr(not(feature = "selftest"), expect(dead_code))]` across `kernel/`
and found seven attributes that had stopped being true. Its "Left owed" named
`wost`, where the form would be a plain `#[expect(dead_code)]` because `wost`'s
dead code is dead in every configuration.

`wost` has no `#[allow(dead_code)]`. So this took the wider question instead:
**every** lint suppression in `wost` and `libs/` is the same kind of assertion,
and none of them was checked either.

There were fourteen. Converting all of them found one stale, and — more
usefully — found the two places where the technique does not apply.

**What was stale.** `#![allow(unreachable_pub)]` on `port::sealed`, the module
holding the sealed-trait marker. The lint no longer fires there: `PortValue:
sealed::Sealed` makes `Sealed` reachable through a public bound, so the trait
is not unreachable and has not been for some time. Deleted. Putting it back
fails `cargo xtask clippy`, which is the break-test.

Two more were live and are now falsifiable:
`missing_debug_implementations` on the Limine glue, and `missing_docs` on
`ps2-keys`' key names, where the comment beside it — "the names are the
documentation" — is the kind of justification that ages well and is worth
keeping honest.

**The first limit: `expect` does not fire inside a macro expansion.**

`boot::entry!` carries ten `#[allow(unsafe_code)]`. They exist so the calling
crate can hold `#![deny(unsafe_code)]` while `wost`-authored code expands
there; each sits on a `#[used] static` with an `#[unsafe(link_section = …)]`.
Converting them to `expect` compiled clean, which looked like a result.

It was not one, and it was measured rather than reasoned:

1. Deleting the `#[unsafe(link_section)]` from one of those statics — the only
   thing on it that triggers `unsafe_code` — and leaving the `expect` produced
   **no unfulfilled expectation**.
2. Adding `#[expect(non_snake_case)]` to a static named
   `__WOST_REQUESTS_START`, which nothing could possibly fulfil, was reported
   by **nothing**.

`unfulfilled_lint_expectations` does not reach into a macro expansion. So an
`expect` there reads as checked and is exactly as unfalsifiable as the `allow`
it replaced — which is **worse than the `allow`**, because the next reader
spends their scepticism elsewhere. All ten are reverted, and the macro's doc
comment carries the measurement rather than this entry alone.

**The second limit: a permission is not a claim.**

`wost/src/lib.rs` carries `#![allow(unsafe_code)]`, the one place the
workspace's deny is lifted (D-67). It converts cleanly and stays `allow`.

`expect` asserts that the lint *does* fire. Here that means "`wost` contains
`unsafe`". The rule this attribute implements is that `wost` is the only crate
that **may** — D-13's framekernel discipline, not a quota. A refactor that
emptied `wost` of `unsafe` would be the best day this design ever had, and an
`expect` would fail the build for it.

The general form, which is what makes this worth an entry rather than a
comment: **`expect` is right where the suppression covers a specific construct,
and wrong where it grants a standing exemption.** D-130's `dead_code`
attributes are the first kind — each names one item that has no caller.
A crate-level `unsafe_code` lift is the second.

**Decision.** Convert the suppressions that cover specific constructs and are
not macro-expanded; delete the one that had gone stale; leave the ten macro
attributes and the crate-level lift as `allow`, each with the reason in the
source rather than only here.

**Rationale.** D-130's argument is right and this does not weaken it: an
`allow` that stopped being needed is silent for ever, and the gate should hold
us to the claim. What this adds is where that stops working, found the only way
it can be — by deliberately breaking an expectation and watching nothing
happen. A technique with an unstated boundary is how a later session gets a
false green, and "convert `allow` to `expect`" reads as universally applicable.

The macro case is the one to remember, because it is invisible: the attribute
is written in `wost`, the lint is evaluated in `kernel/`, and the reason it
cannot be checked is a property of neither file.

**Consequences.**

- `wost` and `libs/` hold two `expect`s where they held `allow`s, one attribute
  fewer than before, and eleven `allow`s that are documented as unconvertible
  with why.
- No check-count change and no image change: this is entirely about what fails
  the build, not what the build produces.
- D-130's boundary is stated. Anyone converting a macro's attributes now finds
  this before spending a session on it.

**Left owed.** The eleven remaining `allow`s in `wost` are unfalsifiable and
stay that way. The macro ten could be made checkable by moving the statics out
of the macro and into `wost` proper — which rust-lang/rust#47384 forbids, since
`#[used]` statics inside an rlib can be dropped wholesale by the linker. That
is the same constraint D-67 recorded, so the ten are as good as they get until
that changes.

## D-132 A public function nothing calls is a check nobody wrote — Status: Accepted (2026-08-25), amends D-130

**Context.** D-129 swept `wost` for `pub fn`s nothing calls and found two. The
same sweep over `libs/` is a different question with a different answer, and
worth stating why: `kernel/` is a binary crate, so `dead_code` already reports
everything unreachable in it — that is what D-130's twenty-six attributes were
suppressing. A `libs/` crate is a library, so its public surface is reachable
*by definition* and the compiler will never say a word about it. Nothing in the
gate looks at it at all.

Two were dead. One is dead weight; the other is the interesting one.

**`Mixer::is_identity` had no caller anywhere, including in tests.** It exists
to ask exactly one question, and D-95 is why: the kernel derived its own mixer,
got `Mixer(0)` for the first table, and every handle in that table came out
unmixed — the first one was the literal integer three. `for_table` was written
to fix that, `is_identity` was written to name the property, and then **no test
in the crate called `for_table` at all.** Every test uses `Mixer::new` with a
hand-picked non-zero seed. So D-95's fix has never been exercised, and the
helper standing next to it made it look as though it had been.

And the property does not hold. `for_table` reads "distinct and never the
identity", and the arithmetic says almost: the multiplier is odd, so the
multiply is a bijection mod 2³², so **exactly four `id`s land in
`0..=RESERVED`** — which is precisely the range `Mixer::new` clears to zero.
Four table numbers out of four billion get a mixer that mixes nothing:

    3016240323  3356813644  3697386965  4037960286

No kernel counting tables upward reaches one. That is also exactly what was
true of `Mixer(0)` until the day the kernel derived one.

**Options.**

1. **Correct the doc to "almost never".** Honest, and leaves a mixer that can
   be the identity in a codebase whose reason for having a mixer is D-95.
2. **Force a bit on after the mask**, so the result cannot be zero. Cheap, and
   it breaks the bijection — two `id`s differing only in that bit would collide,
   doubling the collision class for every table rather than for four.
3. **Divert the four.** Keep the bijection, and give the identity's preimages
   the multiplier itself.

**Decision.** Option 3, with the two constants named — `MULTIPLIER` and
`ADDEND` — and a test that derives the four `id`s rather than listing them.

The test computes the multiplier's inverse mod 2³² by Newton's iteration, five
steps, asserts it *is* an inverse, and uses it to name the four preimages. A
hardcoded list of four magic numbers would be a constant that only equals
itself, which is the thing D-124 was about; a derived one fails if anybody
changes the multiplier.

**Rationale.** The bug is four-in-four-billion and the finding is not. What
this is really about is that D-95's property had a *name* and no *check*, and
the name is what made it invisible: a reader who greps for the concern finds
`is_identity`, sees a function whose whole purpose is to answer it, and
concludes it is covered. An unused helper is worse than an absent one, because
it reads as evidence.

That generalises past this instance. `dead_code` is a real check and it stops
at a crate's public boundary, so every `libs/` crate has a surface the gate does
not look at — and this project puts its most careful arithmetic there precisely
because `cargo test` can reach it.

Why not option 2: the collision structure is already four-to-one from the mask,
and it does not matter, because a table's identity is compared separately —
`entry.owner != self.id` is what refuses a handle from another table, not the
mixer. The mixer's job is to remove the accidental case. Making it worse for
every table to fix four of them is the wrong trade, and it would have made the
new "two table numbers do not share a mixer" test fail, which is how the trade
became visible rather than theoretical.

**Consequences.**

- `for_table` never returns the identity, and its doc no longer says something
  the code does not do. Broken and watched go red (D-28): the old body fails
  `no_table_number_gets_a_mixer_that_mixes_nothing` on all four `id`s.
- A second test records the bound the first one does not: table numbers spread
  across distinct mixers, *four-to-one at the mask*, which is a property of the
  format rather than a fault.
- `Flags::intersects` in `libs/x86-paging` is deleted. Zero references, tests
  included; `contains` is what every caller wants and `is_write_execute` is
  where the one composite predicate lives (D-12).
- `xstate::Layout::size`'s doc said "never less than `HEADER_BYTES`" two lines
  above `is_consistent`, the function written to catch the case where it
  returns zero. It now says what it does and points at the invariant.

**Amendment to D-130.** D-130 concluded that converting `#[allow]` to
`#[expect]` makes an unfalsifiable claim falsifiable. Two limits were measured
afterwards and are recorded in D-131; both are real and neither was foreseen
here. In short: an `expect` inside a **macro body** is reported by nothing, so
it is *less* honest than the `allow` it replaces — it reads as checked. And an
`expect` asserts that a lint *does* fire, so it is wrong wherever the
suppression grants a standing exemption rather than covering a specific
construct — `wost`'s crate-level `#![allow(unsafe_code)]` says `wost` is the
only crate that **may** contain unsafe, not that it does, and an `expect` there
would fail the build on the day the crate was finally emptied.

**Left owed.** The `libs/` sweep was done by hand, with a shell loop counting
identifier occurrences, and **a green result from it is not evidence of
anything.** It found two out of roughly four hundred public functions — a good
enough ratio to be worth repeating and a bad enough tool to be worth replacing.
It cannot see a name used only in a doc comment, so it over-reports; it would
miss anything reachable only through a `pub use`, so it under-reports; and it
matches text rather than resolving paths, so two items sharing a name defeat it
entirely. A second pass over `pub const`, `pub static`, `pub type` and the
`pub struct`/`enum`/`trait` declarations came back clean, which says as much
about the tool as about the tree.

D-129's sweep of `wost` was narrower still — top-level `pub fn` only, where both
of these are inherent methods — so the two passes together are not a covering
one. `xtask` is where a real one belongs, beside `unsafety` and `decisions`,
and it is not written. Until it is, `libs/`'s public surface is the one place in
this repository where "nothing reported it" means nothing looked.

## D-133 A premise about every trap arm, kept in a different file from the arms — Status: Accepted (2026-08-25)

**Context.** `thread.rs` opens with the rule the whole tickless design rests
on: *every path that changes who is runnable ends by arming the timer for the
next moment a decision is due, or stopping it when there is none.* D-57
established it and D-93 fixed `unblock` for breaking it.

D-93's fix was conditional:

```rust
if woken > 0 && !interrupt::in_trap() {
    reschedule_timer(Some(time::now_ns()));
}
```

with the reason beside it: *"From a handler that is fine — the trap exit
reschedules (D-54)."* That is a claim about **every arm of `on_trap`**, and
`on_trap` is in a different file. Nothing compared the two, and they stopped
agreeing the moment ring 3 got a gate.

The timer arm ends in `thread::reschedule`. The keyboard arm ends in
`thread::reschedule` when a keystroke woke somebody. **The syscall arm does
not.** `Call::RingEnter` runs `serve_ring`, which runs `Queue::serve`, which
runs `WaitQueue::wake_all`, which runs `unblock` — and then returns an answer
in `rax` and goes back to ring 3. A `ring_enter` that wakes a waiter leaves it
Ready with nothing armed for it, on a kernel where there may be no other timer
running at all (D-57).

Latent, and only just: `wake_all` returns zero when nothing is registered, so
`unblock` is never reached, and nothing blocks on a `Queue`'s wait queue yet.
"Yet" is doing real work in that sentence — a blocking receive on a ring is the
first thing anything built on this ABI will want.

**Options.**

1. **Add `thread::reschedule(frame)` to the syscall arm.** Restores the
   agreement, and leaves the coupling in place for the next arm somebody
   writes. It also changes when user threads yield, which is a scheduling
   change made to fix a bookkeeping bug.
2. **Arm unconditionally in `unblock`.** Deletes the coupling instead of
   repairing it. Costs one APIC write on the two paths that were going to
   reschedule anyway.
3. **Leave it and record the debt.** Nothing is broken today.

**Decision.** Option 2. `unblock` arms whenever it woke anything, trap context
included.

**Rationale.** The bug is not that the syscall arm forgot something. It is that
a premise about a set of call sites was written next to the code that depends
on it rather than next to the sites, so the two could drift with nothing to
notice — and they did. Option 1 restores the agreement and leaves the same trap
set for the next person to add an arm; option 2 removes the reason the two files
had to agree at all.

The cost is one register write on a path that has just taken an interrupt, and
`unblock`'s own comment already stated the rule unconditionally. The `in_trap`
exemption was the thing stopping it obeying, added on a premise that has since
become false.

Option 1 is also the larger behaviour change of the two, which is the opposite
of how it reads: arming a timer that may fire slightly early is smaller than
changing the point at which a user thread can be preempted.

**The check is about the arming, not the running, and that is the whole
difficulty.** A woken thread runs eventually whatever this does — the checking
thread's own yields and sleeps arm timers, and any of them picks it up — so a
check that waited for it to run passes with the bug present. That is the shape
D-124 kept finding, and it is exactly why this survived: *the failure is only
observable on a machine that would otherwise be idle, and a self-test is not
idle.*

So the check reads a counter of wakes-from-a-handler that armed here. The
keyboard is the only handler that wakes anything today, which is what makes a
zero a falsifiable claim rather than an absence of evidence — the check
requires the blocked reader to have woken *and* the counter to have moved.
Broken and watched go red (D-28). The two lines together are the argument, and
they are worth reading in the order the boot printed them:

```
[info ] selftest: the blocked reader woke with the injected key, without polling
[error] selftest: a wake from a handler arms the timer for what it woke FAILED
[error] selftest: verdict v1 FAIL passed=136 failed=1 expected=137
```

**The reader still wakes.** The keyboard arm reschedules a moment later, so the
consequence arrives whether or not `unblock` armed anything — and every check
that asserts the consequence is green on that boot. Only the counter is red.
Any version of this check written against "did the thread run" would have
passed against the defect it was written for.

**Consequences.**

- One machine check, `a wake from a handler arms the timer for what it woke`,
  and one counter, `HANDLER_ARMS`, which was zero for the whole of the kernel's
  life before this.
- `queue.rs`'s `take` no longer fails silently on either of its two unreachable
  paths. `submit` reserves a completion slot and `take` consumes the
  submission; a `None` from `Ring::consume` after a successful `produce`, or a
  `read_at` refusing a range that is inside the shared pages by construction,
  would hold the reservation for ever. The ring then admits nothing more and
  the program waits on a completion that will not come — a permanent wedge,
  which is worse than a panic because nothing can diagnose it. `debug_assert!`
  in both, matching what D-115 already did to `serve`'s twin two functions
  down; the inconsistency was in one file, from one ADR.

**Left owed.** The check exercises the *keyboard* path, because that is the
only wake from a handler this kernel performs. **The syscall path — the one
that was actually broken — is not exercised**, and it is worth being precise
that this is a check that is *constructible and not built* rather than one that
cannot be built: `Bootstrap::ring` is an `Arc` since D-126, so a kernel thread
can hold the same ring a program does; `queue.rs` already carries
`reserve_for_test`, `post_submission_for_test` and `publish_tail_for_test`, so
a `wait_for_test` is in the file's idiom; and the program that rings its own
doorbell exists. What is missing is the orchestration to get a kernel thread
blocked on that ring *before* the program rings it, which is a race the check
would have to win rather than assume. Those are different debts and the second
kind is the one nobody ever pays.

The redundant arming on the timer and keyboard paths is real and unmeasured. It
is one write to the APIC's initial-count register, on a path already committed
to a `reschedule` that writes it again; if that ever shows up in a latency
measurement, the fix is a check that the deadline actually changed, not a
return to the exemption.

## D-134 A mutation is caught when *its* check goes red, not when the boot does — Status: Accepted (2026-08-25)

**Context.** D-119 made breaking the checks a task: apply a known defect, boot,
require something to go red. The runner asked `boot_is_red` — whether the
verdict was `FAIL`. Not which check failed. `Mutation::breaks` names the
property, and it was printed in a progress line and compared to nothing.

So "caught" meant *some* check went red. A peer session auditing the table
found the gap, and found it live rather than hypothetical:

- **Mutation 4** — `a reclaimed frame is cleared before the kernel sees it` —
  reddens two checks: `a user page comes back cleared`, its own target, and
  `kernel memory cannot be published to ring three`, which is **mutation 3's**.
  Delete mutation 4's target tomorrow and it goes on printing "caught" for
  ever, off the back of another entry's check.
- **Mutations 2 and 10** overlap on ten checks between them. Either could lose
  its own target and never say so.
- Mutation 2 reddens thirteen checks and mutation 10 eleven. Neither is wrong;
  "caught" tells you almost nothing about either.

This is the sentence in `mutate.rs`'s own module doc — *"a mutation that
silently stopped testing anything, which is precisely the failure mode this
whole file exists to catch"* — applied to the expensive tier, which the file
built the cheap tier to protect it from. It is also D-124's shape exactly: a
check measuring a description rather than the thing.

**It made a published claim false.** The sweep run earlier the same day was
reported as "seventeen mutations, seventeen caught", meaning each defect broke
the property it names. What the tool established was that seventeen boots went
red. The weaker statement was the true one, and the gap was a shortcut in the
runner.

**Options.**

1. **Record the limit** in D-119 and move on. Cheap, honest, and leaves the
   sweep's output meaning less than it says. Proposed on the grounds that no
   mutation was currently falling through — which mutation 4 refutes.
2. **A cheap tier only**: assert in `cargo test` that each named check exists.
   Milliseconds, at every commit. Catches a renamed or deleted target; cannot
   see which check a boot actually reddened.
3. **Both tiers.**

**Decision.** Option 3, and the pricing that settles it is not
boots-against-a-hypothetical: the twenty boots are the *verification*, and
they upgrade every future sweep's output permanently. The break-test is one
boot, not a second sweep.

`Mutation` gains `reddens: Caught`:

```rust
pub enum Caught {
    /// The exact `report.check("…")` name that must appear as `FAILED`.
    Check(&'static str),
    /// Nothing names it: the defect kills the machine before the verdict.
    /// Carries *why*.
    Panic(&'static str),
}
```

`breaks` stays, and that is deliberate. **Only six of the twenty are check
names**; the other fourteen are paraphrases, and they read better in a progress
line than the checks they stand for. Collapsing to one field would mean either
fourteen worse progress lines or fourteen renamed checks — a test bending the
code to satisfy itself.

`Caught::Panic` carries its reason because a bare variant is the way to dodge
naming a check. Two entries use it: without the fixup a faulting user copy is
terminal, so the kernel dies before any verdict; and logging from trap context
trips `ThreadLock`'s assert, which is the defect being a defect.

`no_two_mutations_name_the_same_property` still compares `breaks`, not
`reddens`. Mutations 4 and 14 both name `a user page comes back cleared` —
one check, two properties, two defects, because that check asserts both the
reclaim path and the drop tally. Forcing them apart would push someone to split
a good check to satisfy a test.

**The check names come from the machine.** `qemu::SerialObservation` now
records every `selftest: <name> FAILED` line and `correlate_selftest` puts them
in the error, so the runner compares against the kernel's own report rather
than anything it inferred.

**Rationale.** The two tiers catch different things and neither subsumes the
other. The cheap one catches a target that no longer exists — invisible to the
sweep, because the mutation still boots and still reddens something. The
expensive one catches a target that exists, is fine, and *is not the one that
failed* — invisible to any host test, because no amount of string matching
knows which check a boot reddened.

**What the first strict sweep did, and it is the argument for running it.** It
reported **eighteen survivors** on boots that had each reddened exactly the
right check. Serial lines arrive with a trailing `\r`, so `strip_suffix(" FAILED")`
matched nothing and every name list was empty. The fix is one `trim_end`. Had
this been reasoned about rather than run, the table would have shipped with a
comparison that never matched — a stricter-looking check that was strictly
worse, since it would have reported survivors for defects that were caught.
That is the same lesson as D-131's macro measurement, a week's worth of
sessions apart: the tool's behaviour is not deducible from its description.

**Consequences.**

- `cargo xtask mutate` prints *"every mutation was caught by the check it
  names"*, and that sentence is now true. Twenty for twenty at `3df6281`.
- Two host tests: every named check exists in `selftest.rs`, and every `Panic`
  says why. Both broken once and watched go red.
- `parse_failed_check` has its own test, carriage return included, because that
  is the character that cost a twenty-boot sweep.
- Mutation 10's target was **wrong and was corrected by measurement**. It was
  mapped by reasoning to `two spaces map one address to different memory` — and
  that check reads through `translate_in_root`, which walks a root it is handed
  rather than the one `CR3` holds, so it answers the same either way and cannot
  see the defect at all. It is `a second thread crosses onto its own stack`.
- `is_a_verdict` is gone; `boot_reports` distinguishes the two verdicts itself.

**Left owed.** What survives both tiers: a mutation whose named check still
exists, is still named in the log, and has quietly stopped being able to fail —
with the mutation's own check reddening for some *other* reason. Nothing short
of a per-check causal verdict closes that, and nothing here pretends to.

The eleven-and-thirteen-check blast radii of mutations 2 and 10 are not a
defect but are worth knowing: a mutation that reddens a third of the suite is
testing the boot as much as the property, and a narrower defect in the same
place would be a better entry.

## D-135 The boot that was every phase's definition of done checked nothing — Status: Accepted (2026-08-25)

**Context.** Three adversarial audits were run against Phases 0–5 to test the
claim that they were finished and polished. The soundness pass returned the
result that matters most — *no path from safe `kernel/` code, or from ring 3,
to undefined behaviour is reachable on today's kernel* — and the other two
returned roughly forty checks that cannot fail and a long list of prose the
code contradicts.

Two of those findings are about the **verification apparatus itself**, and
they are the ones this entry is for. A defect in a check costs one property; a
defect in the thing that runs the checks costs all of them.

**The VMware boot ran none of the checks.** `Task::Vmware` built its image
with `image::build(&paths, profile, &[])` — an empty feature list, so no
`selftest` feature, so none of the 137 checks were in it. `vmware.rs` mentions
neither `selftest` nor `verdict`; it wraps a disk and prints a path.

D-18 makes a VMware boot **every phase's definition of done**. `AGENTS.md` says
it "is not decoration — it reaches SMEP/SMAP/UMIP, the compacted `XSAVEC` path,
a real x2APIC and a processor that reports `MD_CLEAR`, none of which the gate's
QEMU does." Every word of that is true about the *machine*. None of it was
being checked *there*. Five phase closes cited that boot as evidence, and what
it evidenced was that the shipped kernel comes up and settles — which is
`boot-check`, on hardware.

**One character could disable the mutation sweep, and the gate would stay
green.** `qemu::SELFTEST_PASSED`/`SELFTEST_FAILED` are 33 and 35: QEMU's
`(value << 1) | 1` applied to `debug_exit.rs`'s `PASSED = 0x10` and
`FAILED = 0x11`. They were "kept in step" by a comment saying so.
`the_serial_contract_matches_the_kernel_source` reads `selftest.rs`, `main.rs`
and `panic.rs` — not the file that writes the port.

Change `FAILED` to `0x12` and `cargo xtask ci` passes, because every boot in
the gate is a *passing* one and `PASSED` is untouched. But `mutate` can no
longer tell a red verdict from a broken build, so all twenty-one mutations
report "inconclusive" — D-119's tool, and with it D-28's guarantee, switched
off by a hex digit with nothing to say so.

**Decision.**

`cargo xtask vmware` builds with `SELFTEST_FEATURE`. The boot now reports a
verdict on the serial log the way the gate's does, and the task prints the two
things a human must do that the QEMU harness does for itself: press `a` when
the keystroke marker appears — `await_key` allows ten seconds, after which
that one check goes red and names itself — and read the verdict.

The harness's exit codes are **derived** from the kernel's rather than
restated: the test reads `PASSED`/`FAILED` out of `debug_exit.rs` and applies
QEMU's own doubling. A twin that agrees with itself is not a check (D-124).

Both are held by host tests, and both were broken once and watched go red.

**Rationale.** The VMware finding is the more serious and the less obvious,
because nothing about it looked wrong: the task works, the VM boots, the serial
log fills, and a human reading it sees a healthy kernel. What was missing was
the one thing the phase-close ritual claimed to be getting from it. That is
D-124's shape — a check measuring a description — one level up: the *procedure*
was measuring a description of verification.

Why the whole self-test rather than a hardware-only subset: the checks that
would most repay running on real hardware are not knowable in advance. The
compacted `XSAVEC` path and a real x2APIC are the named ones; the value is in
the ones nobody predicted. And a subset needs a second image, a second feature
and a second thing to keep in step, which is what produced the exit-code twin.

**Consequences.**

- The VMware image is a self-test image. It is not the shipped kernel — that is
  `boot-check`'s job, in the gate, on QEMU. A phase close that wants both now
  runs two boots, and should.
- `the_vmware_image_is_built_with_the_checks_in_it` reads the dispatch's source
  and requires the feature. A typed `Kernel::{Shipped, SelfTest}` at
  `image::build` would make the test unnecessary by making the mistake
  unspellable, and is the better fix; this is the one that fits in the commit
  that found it.
- `the_exit_codes_are_the_kernels_doubled` derives 33 and 35 from `0x10` and
  `0x11`. Changing either without the other now fails `cargo test` in
  milliseconds instead of silently retiring the mutation sweep.
- No prior VMware result is retroactively worth more. The Phase 5 close's boot
  showed what it showed; this entry does not upgrade it, and the next close is
  the first one whose VMware line means what the ritual says it means.

**Left owed.** `image::build` still takes `&[&str]`, so any future caller can
make the same mistake in a new place and only the one call site is guarded.
The typed-kernel refactor is the real fix and is not done here.

The audits returned far more than these two — roughly forty checks that cannot
fail, four live code defects, and several ADR citations pointing at the wrong
decision. Those are being worked separately; this entry claims only the two
findings about the apparatus.

## D-136 An accessor nothing reads back, and a mapping that survives being permuted — Status: Accepted (2026-08-25)

**Context.** The Phase 0–5 audit returned roughly forty checks that cannot
fail, verified by mutation rather than by reading: the crate copied aside, the
defect applied, `cargo test` run. Eight of them share a property that makes
them worth a decision rather than eight fixes — **a real defect reaches the
machine and nothing in the tree catches it.** This entry takes the five in
`libs/`, where the whole justification for the crate is that `cargo test`
reaches the arithmetic.

**Two shapes, and the second is the one worth naming.**

*An accessor no test reads back.* `Slot::index`, `SizeClass::index` and
`AccessRange::offset` were each called by production code and by no assertion.
Derived `PartialEq` compares *fields*, not accessors, so a test that round-trips
a value proves nothing about the getter — and each of these had one:

- `fn index(self) -> u32 { self.generation }` survives `handle-space`.
  `kernel/src/handle.rs` does `arena.slots.get(named.index())`, so every handle
  resolves to whichever slot the generation names, with the generation check
  still passing.
- `{ self.index + 1 }` survives `slab-layout`. `wost/src/mm/heap.rs` indexes
  `self.free[class.index()]` — the largest class runs off the array, a panic
  inside the allocator.
- `{ self.end }` survives `mmio-range`. `wost/src/mmio.rs` begins its write
  loop at `offset()`, under three `// SAFETY:` comments citing this type.

*A mapping that survives a consistent permutation.* `trap-cause` decodes a
vector to a `Fault` and `Fault::vector` maps back. Both existing tests route
through **both** directions, so any permutation applied to both satisfies them.
Swap `6 => InvalidOpcode` with `18 => MachineCheck` in `decode` *and* in
`vector` and the suite stays green.

That is not cosmetic. `kernel/src/arch/x86_64/trap.rs` decides whether a fault
is the program's to die of by matching on the **variant**. Under that swap a
ring-3 `ud2` arrives as `MachineCheck`, misses the survivable arm, and panics
the machine: **an unprivileged halt, from a permutation nothing could see.**
Only `Breakpoint` and `PageFault` were pinned to a number.

*And one bound never approached.* No test called `remaining_slice()` with
`vruntime` **past** `deadline` — the ones that looked like they did landed
exactly on it, where `wrapping_sub` gives zero anyway. Deleting the
`slice_is_spent` guard survives, and `deadline.wrapping_sub(vruntime)` then
wraps to near `u64::MAX`, which `kernel/src/thread.rs` arms the tickless timer
with. The running thread is never preempted again.

**Decision.** Each is now pinned against something that is not itself:

- The three accessors are asserted against values chosen to be **pairwise
  distinct**, so no getter can stand in for another and pass.
- `trap-cause` gains all twenty-one `(vector, variant)` pairs **transcribed
  from SDM Vol. 3A table 6-1**, both directions, against literals. The reserved
  gaps — 9, 15, 22–27, 31 — are asserted to stay gaps, so a variant renumbered
  into one is caught too.
- `eevdf` runs a thread one tick past its deadline, then a thousand slices
  past, and requires zero.

All five broken once and watched go red, with the exact mutation the audit
named.

**Rationale.** The accessor shape is the more common and the less visible.
Rust's derived equality makes a round-trip test feel like coverage: encode,
decode, compare — and the comparison never touches the getters that production
code actually calls. The rule that follows is worth stating in general: **a
`pub` accessor is part of the contract and needs an assertion of its own**;
a struct that survives a round trip has proved something about its
representation, not about its interface.

The permutation shape is rarer and worse. Two tests, both real, both testing a
genuine property — self-consistency — and neither testing the property that
matters, which is agreement with the *architecture*. That is D-124's
description-versus-thing at one remove: the pair described each other
faithfully and described the SDM not at all. The general form: **when a
mapping's inverse is also under test, one of the two must be pinned to an
external source**, or the pair can rotate freely.

**Consequences.**

- Five `libs/` crates gain a check each; no production code changed. Nothing
  here was a bug — every mapping and accessor is currently correct. What was
  missing was any way to find out if that stopped being true.
- `trap-cause`'s new test is the longest transcription in the tree, and that is
  the point: the numbers are the architecture's, so they are written out rather
  than referred to.
- The remaining Tier 1 findings — the `verw` counter's adjacency, the
  `xstate` save form, and the leaf-guard detector's read position — are in
  `kernel/` and `wost/` and are handled separately.

**Left owed.** The audit's other ~32 findings are Tier 2 and 3: bounds that are
too wide, observation windows that close too early, and names that claim more
than their bodies test. They are real and none of them is a soundness hole. The
sweep also noted its own blind spot, which is worth repeating: `cargo xtask
mutate`'s mutations touch nine files, and the checks it does *not* reach are
where most of these were found. A mutation table dense where it has already
been used is a mutation table that measures its own history.

## D-137 A stray cursor on every scroll, and three claims written the same day they were false — Status: Accepted (2026-08-25)

**Context.** The prose audit over Phases 0–5 was asked to find comments and
documents that say what the code does not, on the standing that a false comment
is a defect rather than untidiness — the next reader spends their scepticism
elsewhere. It returned a long list. Most of it is numbers and stale
cross-references, handled elsewhere. Four items are this entry's, and the first
is a live rendering bug.

**The console carried its cursor up the screen on every scroll.**
`set_cursor`'s own doc says the cursor is "drawn only while the console is
idle — every write hides it first and shows it again afterwards, which is what
keeps it from being scrolled up the screen as a stray mark."

`write_text` does exactly that. `write_line` did this:

```rust
pub(crate) fn write_line(&mut self, text: &str) {
    self.write_text(text);   // ...ends with set_cursor(true)
    self.newline();          // ...and this is what scrolls
}
```

The line break is **outside** the bracket. So `newline` — the call that
triggers the scroll — ran with the cursor visible, and `copy_rows` moved the
underline up with everything else. Every log line in the kernel goes through
`write_line`, so this happened on every scroll, on every boot, in front of
whoever was watching.

**Three claims that were false the day they were written, two of them mine.**

- `vector.rs` — a commit spliced `SYSCALL` into the *middle* of `KEYBOARD`'s
  doc block, so the ring-3 syscall gate's rustdoc summary read "The keyboard
  controller's line, routed through the I/O APIC", and `KEYBOARD` had no doc at
  all. `missing_docs` cannot see it: both are `pub(crate)`.
- `README.md` — "a check requires this paragraph to name the same set" of
  tasks. No such check exists. The real one compares the *parser* to the *help
  text* and never reads this file, and the sets differ: `clean` and
  `verify-vendor` are named nowhere in it. Written in `4f11e49`, this session.
- `port.rs` — "`pic::silence` is the one caller that does this today", of the
  trap where a failed second claim strands the first permanently.
  `i8042::Controller::new` `?`-chains `DATA` and `CONTROL`: a refused `CONTROL`
  strands `DATA` and the keyboard controller can never be built again on that
  boot. Written in `2402763`, this session, as part of D-129 — which was itself
  an entry about a comment that overclaimed.
- Two ADR citations point at the wrong decision: `paging.rs`'s range-check rule
  cites D-84 (the ring model) where the rule is D-83's, and the README's
  degraded boot cites D-83 where it is D-90's. `xtask/src/decisions.rs` checks
  that a citation *resolves*, not that it resolves to the right thing.

**Decision.** The line break moves inside the bracket, and the two writers
share a `put` that does no cursor handling — so neither can put the cursor back
before the other is done. The claims are corrected to what is true, including
naming the second stranding caller.

**A machine check drives it, and building one required making the console
readable at all.** `Framebuffer::pixel_at` reads through the shadow, so it
costs no uncached device read; `dev::console_probe` returns the pixel, the
packed foreground and the line height under one guard, because three calls
would let a log line land between them. The check writes a screenful of blank
lines — a region whose only possible foreground mark is a cursor, since a glyph
would put marks there legitimately — and requires the row a scroll moved
through to be clear of the foreground colour. Restoring the old ordering turns
it red.

**It runs last, and that is load-bearing.** Placed among its neighbours it
scrolls the console out from under every other console check and turned the
editing check red. A self-test check must be free of side effects on the
running kernel; this one cannot be, so it goes where there is nothing left to
disturb.

**Rationale.** The cursor bug is the answer to a question worth asking about
this whole sweep: what does a *rendering* defect cost, if the kernel is
otherwise sound? It cost nothing but the appearance of the thing — and it
survived five phases, a hundred and thirty-seven checks and six audits, because
**nothing in this kernel could read a pixel back.** `005` even carried
`read_pixel` as a naming example for an accessor that did not exist. The
console was the one subsystem whose output no check could observe, and the one
subsystem with a visible defect. That is not a coincidence.

The three false claims share a shape worth stating: each was written *beside*
the thing it described, by someone looking at it, and each was wrong about a
second instance somewhere else. `pic::silence` really is a caller that strands;
the sentence failed by saying *the one*. A claim of uniqueness is a claim about
the whole tree, made from one file.

**Consequences.**

- `write_line` and `write_text` share `put`. The cursor bracket is each
  writer's own.
- `Framebuffer::pixel_at` is a new `wost` export, and the first way anything
  has had to ask what is on the screen.
- 138 checks. The new one runs last, with a comment saying why it must.
- `port.rs` names both stranding callers. Neither is fixed here: a claim is
  permanent by design, so neither *can* guard against it, and the atomic
  multi-range claim that would let them is owed.
- The two miscitations are corrected. Nothing catches this class — a citation
  that resolves to the wrong decision reads exactly like one that resolves to
  the right one.

**Left owed.** An atomic multi-range port claim, so a caller wanting two ports
either gets both or strands neither. Both callers want it and neither can be
made correct without it.

A citation that names the wrong decision is not detectable by machine, and
there are two hundred citations. This entry corrects the two the audit found by
reading; it does not claim there are no others.

## D-138 A claim that cannot be released must be taken all at once — Status: Accepted (2026-08-25)

**Context.** D-129 recorded a property of the I/O port registry and did not fix
it: a claim is **permanent**. Nothing releases, because a driver owns its
registers for the life of the kernel, and that is right. What it makes wrong is
a caller wanting *two* spans.

Take the first, fail on the second, and the first is claimed by a caller that
no longer exists. Nothing can ever claim it again, so the thing that needed
both can never be built — on that boot or any later attempt within it.

D-129 said `pic::silence` was the one caller with that shape. D-137 found the
second, and it is worse: `i8042::Controller::new` chains `DATA` and `CONTROL`,
so a refused `CONTROL` strands `DATA` and **the keyboard controller can never
be built again**. Both were written as `?`-chains; D-129 changed one to
`.zip()`, which reaches the same outcome more reliably.

Neither could be fixed where it stood. The check and the record are two
moments, and the registry has no undo — so any single-span API leaves the
caller holding something it cannot give back.

**Options.**

1. **Make claims releasable.** `Port`/`PortBlock` gain a `Drop` that frees the
   span. It removes the trap and the reason the registry is simple: a claim
   that can be released is a claim whose exclusivity holds only while someone
   is looking, and every `// SAFETY:` comment in `port.rs` rests on "no claim
   is ever released, so nothing else can cover them".
2. **Have each caller check availability first.** `is_available` then `claim`
   is a TOCTOU with a permanent loss at the end of it, and on one CPU it is
   merely usually right.
3. **Ask for all of them in one transaction.**

**Decision.** Option 3. `PortSpace::reserve_all(&[(port, len)])` resolves every
span against the bitmap *and against the others in the same request* before it
writes a single bit, and records all or none.

The pairwise half is not decoration: `span` only sees what is already
**recorded**, so without it both halves of `[(0x60, 4), (0x62, 2)]` pass and the
caller is told it exclusively owns a range it holds twice.

`wost` gains `claim_pair` and `claim_block_pair` over it, and both callers use
them. `MAX_SPANS = 8` is a ceiling because the crate is `no_std` with no
allocator; a request past it is **refused**, not truncated, since a truncated
reservation is the silent version of the bug being fixed.

**Rationale.** The trap is not in either caller — it is in the shape of the
API. A permanent claim and a per-span interface cannot be combined safely by
any amount of care at the call site, which is why two independent authors wrote
the same defect and why D-129 could name it and not fix it. Moving the question
to where the lock and the bitmap already are makes it answerable; leaving it
with the caller makes it a rule, and D-121 is this project's record of what
rules are worth against shapes.

Why not option 1, at more length: releasing would be the general fix and it is
the wrong one *here*. Every unsafe block in `port.rs` is justified by
permanence — `Port::new` is sound because nothing else can ever cover the port.
A `Drop` turns that into a lifetime argument about a value that safe code can
`mem::forget`, and buys nothing this kernel needs: no driver in it ever gives a
register window back.

**Consequences.**

- `libs/port-space` gains `reserve_all` and `MAX_SPANS`, with four host tests:
  a refused pair strands nothing, a granted one takes every span, a request
  overlapping itself is refused, and empty or oversized is refused.
- `wost::port` gains `claim_pair` and `claim_block_pair`. Single-port `claim`
  and `claim_block` are unchanged and remain correct for callers wanting one.
- A machine check drives the property against a port the serial driver really
  owns, so the refusal is real rather than arranged: the pair is refused, the
  free half is still claimable afterwards, and claiming it succeeds exactly
  once. Reverting `claim_pair` to chained reservations turns it red.
- D-129's "Left owed" is discharged. Its other owed item — the two-caller race
  in `pic::silence` — is not: that one needs a second CPU to be reachable and
  belongs with D-56's per-CPU pass, where D-126 also owes `LEAVES_HELD`.

**Left owed.** Three or more spans are supported and nothing asks for them. The
pairwise overlap check is O(n²) over at most eight entries, which is the right
trade at this size and would not be at a hundred.

## D-139 The IDT is not the only door, and a guard nothing could observe — Section: Accepted (2026-08-25)

**Context.** Three latent soundness findings from the Phase 0–5 audit, and one
lesson learned by getting the checks for them wrong first.

**`vector::SYSCALL`'s doc says `0x80` is "the only gate in the table ring 3 may
reach".** True of the table. `syscall` and `sysenter` are *instructions*, not
vectors. Neither `EFER.SCE` nor `IA32_SYSENTER_CS` was ever read, cleared or
asserted anywhere in the tree — and `enable_protections` writes `EFER` by
**OR**-ing a bit into what it read, so whatever the firmware left was
preserved. With either open, a ring-3 `syscall` transfers to ring 0 at `LSTAR`
with no stack switch, at an address this kernel never chose.

Nothing in the tree executes either instruction. So the doors were open, unused
and unguarded — the "a claim nothing enforces" shape of D-124 and D-126.

**Two read-only doors walked a reclaimed space.** `reclaim` empties a space by
setting `self.root = 0`. `may_reach` has carried an `is_reclaimed()` guard
since D-123 — *"Mapping into one would walk frames the allocator has given to
something else"* — and `lookup` and `bind` did not. Both walk `self.root`, so
both read **physical frame zero as a top-level table**; any word there with bit
0 set sends the next level to an arbitrary fifty-two-bit address. `bind` then
installs it.

**And `reclaim` was not idempotent.** A second call on a never-bound space fell
past the `Released` gate and reached `drop(Frame::from_raw(0))` — safe code
violating that function's stated contract, handing **frame zero** to the
allocator. Nothing excludes frame zero from the allocatable set and the
first-fit hint starts there, so it is plausibly the kernel's own PML4.

**Decision.** `enable_protections` shuts both fast-syscall doors, and
`enter_user` **refuses** while either is open — checked at the crossing, not
only at boot, because the claim is about what a particular program can reach.
`lookup`, `bind` and `reclaim` each refuse a reclaimed space, `bind` with a new
`ActivateError::Reclaimed`.

**The part worth recording is that the first two checks could not fail.**

Both were written, both passed, and both stayed green when the fix was removed:

- Physical frame zero reads as zeros on both test machines. An unguarded
  `lookup` walks it, finds no present entry, and returns the same `None` the
  guard returns. A check on the *answer* is green with the guard deleted.
- Limine leaves `SCE` clear and `SYSENTER_CS` is zero at reset, so
  `fast_syscall_doors_are_shut()` reads true whether or not the kernel ever
  wrote anything.

Both are D-124's shape — a check measuring a description — written into the
same session that removed four of them, by the author of the entry that names
the pattern. That is the argument for D-28's *"break it once"* being a
procedure and not a principle: it is not that the rule is hard to believe, it
is that a green check is indistinguishable from a working one until you make it
red on purpose.

The fixes:

- A `RECLAIMED_REFUSALS` tally lives **inside** each of the three guards, so
  removing one cannot move it. The check requires it to advance by exactly
  three.
- The syscall check is **driven**: open a door, confirm the reader sees it
  open, shut it through the kernel's own path, confirm it closed. The middle
  step is what proves the reader is not simply answering `true`.

**And what the second check still does not establish, stated rather than
implied.** The *shutting* is checked — break `shut_the_fast_syscall_doors` and
it goes red. Whether `enable_protections` **calls** it is not falsifiable here,
because the firmware already leaves both shut. That conjunct is a statement
about the machine, not the kernel, and it is kept because it is what a
*different* firmware would break.

**Rationale.** The three guards are one idea: a value that has been emptied
must refuse every door, not the doors someone remembered. `is_reclaimed`'s own
doc already said "a reclaimed space answers nothing and maps nothing" — the
mapping half was enforced and the answering half was prose.

Opening a door in a check is a deliberate risk and worth naming. It is safe
here for a reason that is checked rather than assumed: no program can be
entered while it is open, because `enter_user` refuses — so a failure of this
check stops programs from running rather than exposing one, which the break
test showed by turning two ring-3 checks red alongside it.

**Consequences.**

- 141 checks. Two of them exist only because the obvious versions could not
  fail.
- `cpu::open_fast_syscall_doors_for_test` is a `pub fn` in `wost` that nothing
  but a check should call, following `xstate::write_vector_probe`'s precedent —
  `wost` has no `selftest` feature, so the name carries the intent.
- `ActivateError` gains `Reclaimed`. `reclaim` is idempotent.
- `paging::reclaimed_refusals` is the second tally-inside-a-guard in this
  crate, after `CLEARED_ON_DROP` (D-123). Both exist because the property is
  invisible in the answer.

**Left owed.** `entry_pointer`'s `// SAFETY:` comment claims *"`direct_pointer`
confirmed the direct map covers all `PAGE_SIZE` bytes"*. `direct_pointer`'s own
doc says the opposite — *"Arithmetic only. Whether the map actually reaches
those bytes is the caller's."* The claim is restated at eight sites and is what
made these two findings look guarded. Correcting it is prose, not code, and is
owed.

`Root` is `Copy` while every sibling evidence type in this crate is
deliberately not, and `translate_in_root` never re-checks liveness. Same class
as the two fixed here, not yet closed.

## D-140 A `SAFETY` comment that credited a check nobody performed — Status: Accepted (2026-08-25)

**Context.** `mm::direct_pointer` turns a physical address into a direct-map
pointer. Its own doc is unambiguous:

> Arithmetic only. Whether the map actually *reaches* those bytes is the
> caller's to establish: `FirmwareMemory` checks the memory map, and a `Frame`
> carries it by construction.

Five `// SAFETY:` comments said the opposite — *"`direct_pointer` confirmed the
direct map covers all `PAGE_SIZE` bytes"*, *"which `direct_pointer` confirmed
the map covers in full"* — in `entry_pointer`, both halves of `Segment`'s
copy, and twice in the heap. One of them was in a function that never calls
`direct_pointer` at all.

Nothing was unsound. The real ground was always there and always sufficient:
**the direct map covers every frame the allocator manages**, established at
`mm::init` and sampled by a boot check, and every address these sites pass is a
frame the caller owns. What was wrong was the *stated* reason.

That matters more than it sounds. D-139 found two doors — `lookup` and `bind` —
walking a reclaimed space's root, which is physical frame zero. A reader
auditing those walks arrives at `entry_pointer`, reads that `direct_pointer`
confirmed the coverage, and stops. **The false comment is what made the
unguarded walks look justified.**

**A second finding of the same shape.** `Root` is unforgeable — only
`task::address_space_of` mints one — and that was taken to settle what it
authorises. It does not. A caller can hold one across the thread's exit;
`collect` then reclaims the space and returns the root frame to the allocator,
which may hand it to a **program**. The walk then reads ring-3-authored bytes
as top-level page-table entries: the arbitrary physical read the type exists to
prevent, reached by staleness instead of forgery. Every sibling evidence type
in that module — `Binding`, `MappedStack`, `StackArena` — is deliberately not
`Copy` with a comment saying why; `Root` carried the same class of claim with
none of it.

**Decision.**

The five comments now state the real ground: provenance plus boot-established
coverage, with `direct_pointer`'s actual contribution named as what it is —
arithmetic, and a refusal on overflow.

`Root` records the slot it was minted from and gains `is_live`, which asks the
task table again. `translate_in_root` refuses a stale one. It stays `Copy`:
copying is harmless once the *reading* is guarded, and reading is the only
thing it authorises — which is a different answer from `MappedStack`'s and
worth being explicit about rather than applying a rule.

A `STALE_ROOT_REFUSALS` tally lives inside the guard, and the check drives a
real program to completion, collects it, and requires both the refusal and the
count. Deleting the guard turns it red.

**Rationale.** The lesson is about what a `// SAFETY:` comment is *for*. This
project requires one on every unsafe operation, and the point is not that
something was thought about — it is that the next reader can check the
reasoning. A comment naming the wrong justification passes review exactly as
well as one naming the right one, and is worse than none: it tells the reader
the question has been answered and points them at the answer.

The `Root` half is D-125's lesson a second time — *evidence has a date* — and
it is the third instance this week: `MappedStack` outliving its mapping, a
reclaimed space still answering, and now a root outliving its thread. The
common shape is a value that was true when minted, in a system where the thing
it describes can be **given away**. Any such value needs a freshness question,
and unforgeability does not supply one.

**Consequences.**

- Five `// SAFETY:` comments state their real ground. No code changed for them,
  and that is the point: the comments were the defect.
- `Root` carries its slot; `translate_in_root` re-asks. 142 checks.
- `paging::stale_root_refusals` is the third tally-inside-a-guard, after
  `CLEARED_ON_DROP` (D-123) and `RECLAIMED_REFUSALS` (D-139). All three exist
  because the guard's absence is invisible in the answer.

**Left owed.** Nothing checks that a `// SAFETY:` comment names a real reason;
nothing can. `xtask/src/unsafety.rs` counts the lines and the lints require the
prose to exist. Whether it is *true* is a reading, and this entry is the record
of five that were not.

The audit's other latent items — the handle arena's `take`/`insert` race and
`mem::with_active_space` handing safe code `&mut` the kernel's own space — are
the same family and are not closed here.

## D-141 A slot handed out and not marked — Status: Accepted (2026-08-25)

**Context.** `HandleTable::insert` took the arena's leaf guard **twice**:

```rust
let index = loop {
    reserve_a_slot()?;                      // allocates, so outside the lock
    if let Some(index) = ARENA.lock_leaf().take() { break index; }
};
let mut arena = ARENA.lock_leaf();          // ...and again
arena.slots[index].entry = Some(entry);
```

`take` scanned for `entry.is_none()`, advanced the hint, and returned the
index — **without marking the slot**. So between the two acquisitions the slot
was free by every test the arena can make, and the guard had been dropped, which
restores the interrupt flag.

Two callers interleaving there both receive the same index. The second write
**overwrites a live entry**, and the displaced `Entry` is dropped in place,
under the leaf guard. An `ObjectRef` dropped there can reach `Channel::drop` →
`WaitQueue::wake_all` → `THREADS.lock_leaf()`: the lock-under-a-leaf class
D-121, D-126 and D-128 exist for, and one D-126's runtime detector sees only on
a boot where the race actually fires.

The loser also loses its authority — its handle resolves `NotYours`. That half
fails closed, which is why it would have been a mystery rather than a breach.

**Reachable today: no.** Spawn is single-threaded and no syscall mints a
handle. Live the moment either changes.

**Decision.** `take` becomes `claim(entry)`: it finds the slot **and installs
the entry**, under the one guard that found it. `find_free` is the private
scan. `insert` builds the `Entry` inside the retry loop — `ObjectRef` is an
`Arc`, so the clone is a refcount — and the second acquisition is gone.

The reservation stays outside the lock, because that is the other half of the
same rule: growing allocates, and allocating under this guard is the
`ARENA → HEAP` order D-128 removed.

**Rationale.** The window existed because two things had to be true at once —
"the room is reserved" and "the slot is mine" — and they were established under
separate guards with nothing carrying the first claim across. That is the same
shape as D-138's port stranding: a resource taken in one step and committed in
another, with no way to hold the gap. Where D-138 could not close it at the
call site and had to move the question into the registry, here the fix is
simply to do both under one guard, because nothing between them allocates.

**What the check does and does not establish, said plainly.** It fills a table
past a grow, and requires every handle to be distinct, to still resolve to what
it named, and the live count to match. That is the invariant an overwrite
breaks. It **does not reproduce the interleaving**, and nothing here can: it
needs two threads inside a window between two lock acquisitions, and a check
that spawned racers to try would be flaky — which is worse than a narrower
check honestly described. Breaking `claim` so it hands out a slot it never
fills turns it red, along with two other handle checks.

**Consequences.**

- `Arena::claim` replaces `Arena::take`; `insert` takes the guard once. 143
  checks.
- The retry loop clones an `Arc` per attempt. Attempts past the first are the
  case where another table consumed the reserved room, which D-105 already
  records as expected and rare.

**Left owed.** The interleaving is unreproduced. If a handle-minting syscall or
a second CPU arrives, the honest way to test this class is a stress check with
a bounded flake budget, and that is a decision for whoever adds either.

`mem::with_active_space` still hands safe code `&mut` the kernel's own
`AddressSpace`, whose `Reach::Everywhere` disables both of D-105's guards — so
a caller could reclaim the kernel's tables or unmap the stack it is standing
on. No caller does. Same family as this entry, not closed.

## D-142 Hardware constants checked against the manual, not against themselves — Status: Accepted (2026-08-25)

**Context.** D-136 pinned five `libs/` properties that a mutation could defeat.
The audit's Tier 1 list had three more of the same family, and all three sit
where a wrong value reaches the hardware directly.

**A MADT entry could be read past its end.** `MadtEntries` validates each
entry's length against a per-kind minimum before walking it — 8 for a processor
local APIC, 12 for an I/O APIC, 10 for an interrupt source override. Only kind
1's minimum was pinned by a test. Lower kind 2's to 8 and the suite stays green,
and `next` then reads `entry[9]` for an eight-byte entry: an **index-out-of-bounds
panic on firmware input**, which this module's own doc says it will not do.

**The APIC's base flags could be swapped.** `TYPICAL = 0xFEE0_0900` sets both
`BOOTSTRAP` (bit 8) and `ENABLE` (bit 11), so every test that used it was
satisfied by the two constants exchanged — fifty-seven of them. `Base::enabled()`
would then set the **read-only** bootstrap bit and never enable the APIC.

**The TSS descriptor could be given a scaled limit.** Bits 52–55 of the
task-state descriptor were asserted by nothing. `TASK_STATE_TYPE | 1 << 55` —
granularity — survives, and granularity multiplies the limit by 4 KiB: a
104-byte segment becomes `0x67FFF`, so the TSS's `iomap_base` of 104 lands
**inside** the segment and the CPU fetches an I/O permission bitmap from
whatever follows the TSS in memory. Ring 3's port access would be granted by
whatever bytes happened to be there.

**Decision.** Each is now checked against its manual, as a literal:

- ACPI 6.5 table 5-22's structure lengths, every kind, both directions — one
  byte short is refused, exactly enough is walked without panicking.
- SDM Vol. 3A §11.4.4's `IA32_APIC_BASE` bit numbers, pairwise distinct, and
  none overlapping the address field.
- SDM Vol. 3A figure 8-4's flag byte: 52 available, 53 reserved, 54 default/big
  and 55 granularity all clear, with present and the type nibble asserted set
  so an empty descriptor does not satisfy it.

**Rationale.** This is D-136's rule at a third site and the pattern is now
established enough to state as a convention rather than a finding: **a constant
that describes hardware is checked against the document that fixes it, never
against another constant from the same file.** A value that appears on both
sides of an assertion is a definition; a value that appears beside a literal
from the SDM is a transcription, and a wrong transcription fails the first time
it is compared.

The APIC case is the clearest argument for *pairwise distinct* test values.
`TYPICAL` is a real register value with both bits set, which is exactly why it
could not distinguish them — a test using a realistic input was worse here than
one using a contrived one.

**Consequences.**

- Three `libs/` crates gain a check each, all broken once and watched go red
  with the exact mutation the audit named.
- No production code changed. Every constant was correct; none of them could
  be shown to be.
- `005`'s testing section already carries "write the check from the claim, not
  from the implementation" (D-124). The hardware-constant corollary is now
  demonstrated three times and belongs beside it.

**Left owed.** The audit's Tier 2 and 3 lists hold roughly twenty more of this
family — `ps2-keys`' scancode table anchors 21 of ~89 codes, `exception::name`
never reads `PAGE_FAULT` back, `ring-proto::VERSION` appears on both sides of
every comparison. Each is smaller than these three and none reaches hardware
the same way; they are worth doing and are not done here.

## D-143 A tally beside an instruction, and a detector read too early — Status: Accepted (2026-08-25)

**Context.** Two checks that existed, passed, and could not fail — both about
mechanisms this kernel's security rests on.

**The buffer flush counted its own neighbour.** `VERW` on the return to ring 3
is the MDS mitigation, and D-101 already cost this project a phase by shipping
it unexecuted: reading the *arming* byte says the instruction was compiled in,
not that it ran. D-105's fix was to count in the assembly, "beside the `verw`
itself".

Beside is not the same as because. The stub was:

```
verw word ptr [rsp]
add rsp, 8
inc qword ptr [rip + {clears}]
```

Delete the `verw` line alone and the `inc` still runs, the tally still climbs,
`buffer_clear_is_armed()` is still true, and the check is green with the
mitigation gone. D-101's lesson had been learned one instruction short of
where it needed to be.

**The leaf-nesting detector was read at check eighty of a hundred and
forty-three.** D-126's counter records any lock taken while a `lock_leaf` guard
is held and keeps the **first** such violation. `taken_under_a_leaf()` has
exactly one reader, and it ran two-thirds of the way through the run — so a
violation in any of the sixty checks after it, over `input.rs`'s twelve
`lock_leaf` sites and `dev.rs`'s three, happened after the only read and was
never reported.

**Decision.**

`verw` sets ZF when the selector is writable at the current privilege level,
which the kernel data segment is. The tally is now gated on that flag:

```
verw word ptr [rsp]
jnz 3f
inc qword ptr [rip + {clears}]
3:
add rsp, 8
```

With the `verw` deleted, ZF is whatever `sub rsp, 8` left — clear — so the
increment does not run and the check goes red. The count is now of the
instruction rather than of its position. Both stubs, the trap exit and the
first crossing, carry it.

The detector's read moves to the end of the run, beside the console-scroll
check, with a comment saying why each must be last: one reads a value every
other check can change, the other writes over what every other console check
looks at.

**Moving it found a violation immediately, which is the best evidence it was
in the wrong place.** `a_lock_excludes_interrupts` took two `lock_leaf` guards
deliberately, to check that dropping the inner one does not re-enable
interrupts. The property is real and worth checking; the nesting was the one
direction `lock_leaf` forbids, and it would have masked every later genuine
violation, because the detector keeps the first.

The check now takes the **token-bearing** form for the outer guard and the leaf
for the inner — the sanctioned direction, the one that makes D-47's
`HEAP` → `FRAMES` order sound — which is what it needed anyway: the property
under test is about interrupts, not about lock order.

**Rationale.** Both defects are the same mistake at different scales:
**position mistaken for participation.** An `inc` next to a `verw` is not
counting the `verw`; a detector read before sixty checks is not watching them.
In each case the check was written by someone who had the right idea and
stopped one step short of making the mechanism carry it.

The `verw` fix is worth the awkwardness of a conditional in a hot path — two
extra instructions on the exit to ring 3, one of them not taken — because the
alternative is a mitigation whose only evidence is a number that would be
identical without it. D-101's original defect and this one are the same
sentence with different subjects.

**Consequences.**

- The flush's tally is causal. Deleting either `verw` turns
  `the_return_to_ring_three_clears_buffers` red; it did not before.
- The leaf detector watches the whole run. Its first read found a real
  violation in the checks themselves.
- `a_lock_excludes_interrupts` uses the sanctioned nesting. It tests the same
  property and no longer commits the thing another check is looking for.

**Left owed.** The detector still keeps only the *first* violation, so a
deliberate one anywhere still masks the rest. Nothing needs it today — there
are none — and the honest fix is a small ring of the last few, which is worth
doing when there is a second CPU and the counter has to move per-CPU anyway
(D-126's owed item).

The audit's remaining Tier 2 findings include two more of this family: the
`xstate` save form is dispatched on a flag no check reads, and
`ring_proto::is_consistent` is asserted true at every call site and never
false. Neither is closed here.

## D-144 The save area was sized for one form and written by another's dispatch — Status: Accepted (2026-08-25)

**Context.** `xstate::init` decides whether this processor gets the compacted
`XSAVEC` path or the plain `XSAVE` one, then stores two things: `BYTES`, the
size a save area needs, and `FORM`, the word `save` dispatches on.

Nothing read `FORM`. `main.rs` prints ", compacted" from
`Extended::compacted` — a `CPUID` bool returned by `init` — not from the
dispatch. So storing `FORM = 1` with a compacted `BYTES` writes **past the end
of every save area on the machine**, each of which lives inside a thread's
slot, while the banner goes on announcing the right thing.

They are stored on adjacent lines and agree. What was missing was any way to
find out if they stopped.

**Decision.** Checked **by consequence**: `save_stays_inside_its_area` puts a
sentinel past `bytes()` and requires the save not to touch it. That catches the
disagreement in either direction, and would catch a `BYTES` that was too small
for a reason nobody has thought of yet.

**The first version asserted the form, and was wrong.** It required
`saves_compacted()`, and went red on the gate: QEMU's `-cpu max` reports no
`XSAVEC`, so the kernel correctly chooses the uncompacted path.
`AGENTS.md` names the compacted path as one of the things only the VMware boot
reaches — so a check requiring it is a check about the *hardware*, green on one
machine and red on the other.

That is worth more than the fix. **The property that holds on every processor
is that the area was sized for the form in use** — and a check written that way
is meaningful on both. Since D-135 put the self-test into the VMware image,
this is the first thing that will exercise the compacted path with a check
watching it.

**Rationale.** This is D-124's rule — measure the thing, not the description —
applied where the description was a *flag the code sets about itself*. `FORM`
and `BYTES` are both derived from the same `CPUID` walk, so any check comparing
them to each other, or either to what `init` returned, is comparing two copies
of one decision. The sentinel is outside that circle.

The near-miss is the instructive part: asserting `saves_compacted()` would have
passed review, read like a stronger check, and encoded QEMU's absence of a
feature as a kernel invariant. A check that is red on correct hardware is worse
than no check, because the response to it is to weaken it.

**Consequences.**

- `xstate::saves_compacted` exists and is read by nothing but a diagnostic; the
  check does not use it, deliberately.
- `save_stays_inside_its_area` allocates a 4 KiB aligned area on the stack and
  refuses if `bytes()` plus the sentinel would not fit — so a future `XCR0`
  that grew past it reports rather than overflowing the check itself.
- 144 checks. Halving `BYTES` turns it red.

**Left owed.** The sentinel proves the save did not write past `bytes()`. It
does not prove the save wrote *everything it should have* — a `FORM` of zero
would write nothing at all and pass. `is_enabled()` covers that today because
it reads the same word; a genuinely independent check would compare a restored
register against what was saved, which
`a_vector_register_survives_a_switch` already does for `xmm0` and for nothing
else.

## D-145 Three claims the code had outrun, and one of them lost messages — Status: Accepted (2026-08-25)

**Context.** The prose audit's Tier 4 — comments that assert away a real
behaviour, which makes them defects rather than untidiness.

**A closed channel lost what it was sent.** `receive_blocking`'s doc says
`None` means "the far end closed with nothing left to read". The loop took a
message, then tested orphanhood, in **two separate lock acquisitions with
interrupts on between them**. A peer that sent *and then* closed in that window
got `None` returned with its message sitting in the queue — a lost message,
reported as a clean close, to a caller told that cannot happen.

**`await_key` claimed two bounds, and neither reached its inner loop.** The doc
says "doubly bounded: a deadline on the clock, and an iteration cap". Both are
on the outer `for`. Inside it the drain was `while let Some(event) =
keyboard.next_event()`, and `next_event`'s sixteen-byte budget is *per call* —
refreshed every iteration. A partially wedged controller that keeps a byte
latched with its status bit set spins there for ever, holding `KEYBOARD`'s leaf
guard, under a doc claiming it cannot. Selftest-only, so it could wedge the gate
rather than a shipped kernel.

**The module doc promised every event was queued.** `Events::push` drops on a
full sixteen-deep ring and counts it. Fast typing at a busy reader loses
keystrokes, which is a reasonable design and the opposite of what was written.

**And two `# Errors` blocks listed what could not happen or omitted what could.**
`Channel::send` listed `OutOfMemory`, which it cannot return — the inbox is
reserved to `DEPTH` at construction precisely so `push_back` never reaches the
allocator under a leaf guard, and the body's own comment three lines down says
so. `elf64::Elf::parse` omitted `WrongMachine`, which it checks.

**Decision.** The orphan arm drains rather than refuses: `return self.receive()`
makes the documented sentence true in both orderings. The drain takes a cap of
its own, two batches per pass, so a controller with more to say gets the next
pass and one that never stops saying it does not get the loop. The two docs are
corrected to what the code does.

**What the check establishes, and what it does not.** It pins the **ordering** —
draining before refusing. Test orphanhood first and it goes red. It does **not**
reproduce the window, and cannot single-threaded: the fixed line is reachable
only when a send lands between two acquisitions, so removing that line alone
leaves the check green because the take at the top of the loop finds the
message. That is written into the check rather than left for an audit, which is
the second time in this sweep a fix has landed with a check that pins its
neighbourhood rather than its point (D-141 was the first).

**Rationale.** All three prose defects share a shape: the comment describes the
**intent** of the surrounding code and the code implements it *almost*. Nobody
wrote "doubly bounded" carelessly — two bounds are there, in the loop the
author was looking at. The inner loop was added later, by someone who read the
doc and believed it.

That is the argument for treating a false comment as a defect rather than
untidiness, and it is stronger than "the reader is misled": the reader here is
the next author, and the comment is what tells them the hazard is handled.

**Consequences.**

- 145 checks. `receive_blocking` cannot report a clean close over a queued
  message in any ordering.
- `await_key`'s drain is bounded. Its doc says three bounds and names what each
  one catches.
- Two `# Errors` blocks match their bodies. Nothing enforces that they stay
  matched, and nothing can — `# Errors` is prose about a `Result`, and the
  compiler has no opinion about which variants a function can produce.

**Left owed.** The two-thread window in `receive_blocking` is unreproduced, as
is D-141's arena race. Both need concurrency this kernel does not yet have, and
both would need a flake budget to test — a decision for whoever adds a second
CPU, alongside D-126's `LEAVES_HELD` and D-129's `pic::silence` race, all four
of which are waiting on the same thing.

## D-146 Numbers in prose, checked against the code they describe — Status: Accepted (2026-08-25)

**Context.** The prose audit's Tier 2 was a table of thirteen numbers that
disagree with the code beside them. None is a bug. Collectively they are the
reason a reader stops trusting the prose, which is the asset this project has
been spending three days building.

The ones worth naming:

- A wait was "generous beside a **30 ms** slice". `SLICE_NS` is **3 ms** — the
  wait is ten times more generous than advertised, which is the safe direction
  and still wrong.
- `QUIET_MS = 500` was "**five** times the old tick's period". The tick was
  100 Hz, so it is **fifty** — and the line four above already says *"a number
  that would have been fifty"*.
- `FrameSpace` "occupies **2 MiB** of `.bss`". Two arrays of 65,536 `u64` is
  **1 MiB**, which is what `wost/src/mm/mod.rs` calls "one megabyte" eight
  files away.
- "Sixty-four extents of **twenty-four bytes** is **a page and a half** of
  stack". An `Extent` is 32 bytes, so it is 2 KiB — **half** a page. Both
  halves wrong, in opposite directions.
- "**Four** ways to ask for something that is not a run of owned pages, and
  four refusals" — the body asserts **six**.
- The emitted user program was "**twenty-two bytes** of machine code" before it
  grew three conditional branches and two more behaviours; D-116 says "about a
  hundred" of the same artifact.
- `user::spawn` "called **three** times a boot" — there are **six** spawn
  wrappers, self-numbered "The first" through "A sixth".
- "**three** doors into the higher half" — `may_reach` has five call sites and
  `map_owned` is a fourth door.
- `007` gives host-test counts of nineteen, eleven and seventeen for crates
  that now have forty-three, fourteen and thirty-one — and `007` charters
  itself as "the description of what exists".

**And one that is not an error.** D-52 works through an arena of sixteen 16 KiB
stacks costing 256 KiB. D-113 raised the stack to 32 KiB, so it is 512 KiB. An
ADR records what was decided when it was decided; the numbers stay and the
entry gains an amendment pointer, which is what D-45 and D-51 carry beside it
and this one did not.

**Decision.** Every number is corrected to what the code does, except D-52's,
which is amended rather than rewritten. Where a count will keep drifting — host
tests in a growing crate — the prose says "forty-odd" rather than a figure that
is wrong the next time someone adds a test.

**Rationale.** These are individually trivial and that is the point: none would
justify a commit on its own, so none was ever fixed, and thirteen of them is a
document that reads like a description and is not one. The project's own
standard is `007`'s: *"the description of what exists"*. A count is the easiest
claim to check and the first one a sceptical reader tries.

The "forty-odd" choice is deliberate and worth stating. A precise number in
prose is a hostage: it is right on the day and wrong within a week, and the
person who makes it wrong is adding a test, which is the behaviour the number
exists to encourage. An approximation that stays true is worth more than a
precision that does not — **except** where the number is load-bearing, where
the answer is not prose at all but a check, which is what
`EXPECTED_CHECKS`, `MEASURED` and the sysdep tally already are.

**Consequences.**

- Thirteen corrections across `selftest.rs`, `frame-space`, `mm`, `user.rs`,
  `program.rs` and `007`.
- D-52 carries an amendment pointer to D-113.
- Nothing enforces any of this. The numbers that *are* enforced are the ones
  that were made into constants and checked; the rest are prose, and prose is
  audited by reading.

**Left owed.** The audit's numeric sweep did not reach `docs/008`–`012` — those
were covered for identifiers only. Counts and sizes in `011` and `012` are
unaudited, and `012` is still untracked.

## D-147 Four holes in the things that check everything else — Status: Accepted (2026-08-25)

**Context.** The audit's remaining findings about the *gate*. A defect in a
check costs one property; a defect in what runs the checks costs all of them,
which is why these are worth an entry after the code is already fixed.

**The crate boundary was inferred, never asserted.** `no_libs_crate_has_any_unsafe`
measures unsafe **blocks** and requires zero. That is right for the budget —
`unsafe fn` and `unsafe impl` are declarations, not operations. But it leaves
the boundary itself unchecked: a `libs/` crate that dropped
`#![forbid(unsafe_code)]` and grew an `unsafe fn` has zero blocks and passes.
D-13's framekernel claim is that `wost` is the only crate that may write
unsafe, and nothing tested that the other nineteen still said so.

**`ci` could quietly lose a step.** Delete `vendor::verify(paths)?` from its
body and every host test stays green, because each step is also its own task
and the tests exercise the tasks. `ci` is the only gate this project has
(D-26); a step leaving it is the gate shrinking with nothing to say so. Only
the *boot count* was checked, and only because the README states it.

**`kernel/linker.ld` cites two decisions and was scanned by nothing.**
`decisions.rs` walked `.rs`, `.md`, `.toml`, `.h`, `.cpp`, `.tla` and `.cfg`.
The linker script names D-45 and D-21, and is the file whose properties D-40's
read-back exists to check.

**And D-134's own defect had a second door.** `boot_reports` matched the failed
check's name as a **substring** of the `"; "`-joined list. A future check whose
name contains another's would satisfy that other one — which is exactly the
"caught by something, not by its own check" hole D-134 was written to close,
re-entered one layer down.

**Decision.** Each is now checked:

- Every `libs/` crate's `lib.rs` must contain `#![forbid(unsafe_code)]`,
  `kernel/src/main.rs` must `deny`, and `wost/src/lib.rs` must `allow` — the
  last because it is what makes the other two mean anything. Checked as text:
  the attribute is the assertion, and a build that compiles proves only that
  nothing has violated it *yet*.
- `ci`'s body must contain all nine of its steps, read from `tasks.rs`'s source
  the way the boot count already is.
- `.ld` joins the scanned extensions.
- `boot_reports` splits the list and compares whole elements.

All four broken once and watched go red.

**Rationale.** Three of these are the same shape as the defects they guard
against, which is the observation worth keeping. The unsafe counter measures a
consequence of the boundary and was taken for the boundary. `ci`'s coverage was
inferred from its steps existing as tasks. The mutation runner's substring match
was a *description* of the check name rather than the name.

The fourth is different and simpler: a file extension nobody added to a list.
It is here because the cost of finding it was an audit, and the cost of
preventing it was one word.

Why text rather than a build for the boundary: `#![forbid(unsafe_code)]` proves
itself at compile time *only while something would violate it*. A crate with the
attribute removed and no unsafe compiles identically, and the next author adds
the unsafe. The attribute is a statement about the future, so it is checked as a
statement.

**Consequences.**

- Four host tests, all milliseconds, all in the gate.
- `ci`'s step list is now stated in two places — `tasks.rs` and the test — and
  they must agree. That is a transcription, and it is the right trade for the
  only gate there is.
- The mutation runner compares whole names. The 29-mutation sweep is unchanged
  by it, which is the expected result and not evidence the fix was unnecessary.

**Left owed.** `ci`'s step list in the test is a transcription and will need
updating when a step is added — which is a reviewable line in a commit, the
same mechanism as `MEASURED` and `ABSENT`. Deriving it instead would mean
parsing Rust, which is the thing this project keeps declining to do for good
reasons.

The audit's remaining Tier 3 findings — `ps2-keys` anchoring 21 of ~89
scancodes, `exception::name` never reading `PAGE_FAULT` back,
`ring_proto::VERSION` on both sides of every comparison, the TLA+ config's
unread `INVARIANTS` block — are the D-136 and D-142 family and are not closed
here.

## D-148 The model could check nothing and report success — Status: Accepted (2026-08-25)

**Context.** The last of the audit's Tier 3 findings, and one of them is worse
than its tier suggests.

**TLC checks what `INVARIANTS` names, and nothing checked that it names
anything.** `models/Ring.cfg` lists five. Empty the block and TLC still reports
*"Model checking completed. No error has been found"* — because a run with no
invariants cannot find one — so `cargo xtask model-check` passes with nothing
checked. The config's own comment says the block exists so "a failure is
reported by the name that says what went wrong"; nothing established that a
failure could be reported at all.

D-84 made the ring protocol modelled before it was written, and D-112's
counterexample run is what proved the model could fail. That was a one-time act,
at the moment the model was written — the same shape D-119 made a task for.

**Three constants that only equalled themselves.** `ring_proto::VERSION`
appeared on both sides of every comparison — the kernel writes it into the
header, the checks compare the header against it — so `= 7` survived the whole
workspace, on a field whose entire purpose is to be read by a *program*.
`exception::name(14)` was spelled as a literal on both sides, so `PAGE_FAULT`
was read by nothing and could have named any vector; `COUNT` was satisfied by
anything at or above 31. And `ring_proto::is_consistent` — the oracle six
machine checks use, including the one that catches a lying completion head —
was asserted *true* at every call site and never false, so `{ true }` survived.

**Decision.** Each is pinned against something outside itself:

- A host test requires every invariant the model defines to be named by the
  config, and the block to be non-empty. Milliseconds, in `cargo test`, so it
  runs at every commit while the model check itself needs a JVM and is
  deliberately not in `ci` — which is the two-tier argument `mutate.rs` makes,
  applied to the other tool that lives outside the gate.
- `VERSION` is asserted to be 1, as a literal.
- The exception constants are used to *index* the name table, so a wrong
  constant gives the wrong name, and `COUNT`'s neighbours are checked unnamed.
- `is_consistent` is driven to the edge — submit until the ring is full,
  asserting the invariant at every step and after the refusal — so it is
  exercised where it could say no rather than only where it says yes.

**Rationale.** The model finding is the one worth remembering, because the
failure mode is *silence that reads as success*. Every other check in this
project fails loudly when broken; a model with no invariants completes, prints a
green line, and reports a state count that looks like work. D-112 verified the
model could produce a counterexample once. What was missing was the standing
version of that — and the cheap tier is enough, because the way it breaks is
deletion, not subtlety.

The three constants are D-124's rule at its most literal, and their being Tier 3
is right: none reaches hardware, and `VERSION` is the only one a *program* can
observe. It is also the one that would be hardest to notice, since a wrong
version is not wrong until something else is reading it.

**Consequences.**

- Four host tests. All broken once: the emptied `INVARIANTS` block, a renumbered
  `PAGE_FAULT`, a bumped `VERSION`, and `is_consistent` returning `true`.
- The model's invariant list is now stated in two places — `Ring.cfg` and the
  test — and they must agree. As with `ci`'s step list (D-147), that is a
  transcription and the right trade for a tool the gate cannot run.

**Left owed.** The remaining Tier 3 findings are smaller still and none is
closed here: `ps2-keys` anchors 21 of about 89 base scancodes, so a swap of two
unanchored keys survives; `libs/timebase`'s round-to-nearest survives becoming
round-up for the multipliers its tests use; `libs/pixel`'s `span` is only ever
exercised where `pitch == width * bytes_per_pixel`, so a padded framebuffer
would map short. Each is a real hole in a real check and each needs a table of
values from outside the crate, which is a session's work rather than an hour's.

## D-149 The ABI is extended before the filesystem is written — Status: Accepted (2026-08-25)

**Context.** `000`'s roadmap makes Phase 6 "VFS + storage": a VFS over handles,
tmpfs and devfs, and a read-only third-party format over NVMe. `013` was
drafted as a separate "Phase 6.5" to follow it, holding three constraints on
what a program may ask for.

The ordering is the decision, and it is not cosmetic. **A VFS is the first
subsystem this kernel has with genuinely blocking operations.** Everything
before it either cannot wait — the clock, `exit`, `tcb_set` — or waits on
something the kernel already owns. `read()` waits on a device.

**Options.**

1. **VFS first, platform after**, as the roadmap reads. Fastest to a machine
   that can open a file.
2. **Platform as its own phase, VFS after.** Cleanest separation; the machine
   still cannot store anything at the end of it.
3. **One phase: the ABI extensions as slices 1–3, the filesystem on top.**

**Decision.** Option 3. Phase 6's first three slices are `013`'s work —
completions that carry handles, the time page, what happens to an operation
whose program has retired — and the filesystem is written against them, ring
native from its first `read`.

**Rationale.** Build `read()` synchronous and add the ring form afterwards and
the kernel has two doors to the same operation for ever. That is not a
hypothetical: it is what `011` records Phase 5 avoiding by freezing the handle
table and the queue ABI *before* writing their callers, on the argument that
both get frozen by their first caller. The queue's first caller echoed a word
back. Its second waits on a disk, and it is the second that fixes the shape.

The cost is a slower start — three slices before anything is stored — and it is
the same cost D-19 charges for every charter.

**Consequences.**

- `013` stops being a phase and becomes Phase 6's first three slices. Its own
  framing said it was "not a phase in the sense the others are"; this makes
  that true rather than apologetic.
- Phase 6's charter is written after `012-research-notes-4`, and covers both
  halves.
- Phase 7 opens against an ABI that has already met something that waits.

**Left owed.** Nothing here says what the VFS *is* — the handle-per-file shape,
tmpfs, devfs, and whether the third-party format is FAT32 or ext2 are the
research pass's questions and the charter's answers.

## D-150 No blocking-only door, as a project rule — Status: Accepted (2026-08-25)

**Context.** `013`'s first constraint: every operation that can wait gets a ring
form. The question left open was whether that binds Phase 6 or the project.

**Options.** A phase rule is weaker and gets re-argued. A project rule is
stronger and forecloses a synchronous door somebody may later want for an
operation that only *looks* blocking.

**Decision.** The project. **Every operation that can wait has a ring form, and
no operation is reachable only by a call that blocks.**

The carve-out that makes it liveable already exists and is not an exception:
**a door that cannot wait may stay direct.** `exit`, `tcb_set` and the clock
are all such doors today. The rule is about waiting, not about syscalls.

**Rationale.** The reason to make it a project rule is the reason it would be
re-argued: the first person to want a synchronous `stat` will have a good
local argument, and granting it costs the property that makes the ring worth
having. D-117 caps this machine at sixteen concurrent programs, so
thread-per-outstanding-operation is already wrong here; and any userspace that
multiplexes work onto fewer threads than it has work dies against a
blocking-only door, which is what every language runtime with a scheduler
does.

Stated as a rule rather than a preference because the alternative is not "we
will think about it" but "the second caller decides", and the second caller
will be whoever is in a hurry.

**Consequences.**

- Phase 6's `read`, `write` and `open` are submissions. A blocking read is a
  submission plus a wait, which is also what makes it interruptible by a
  signal, since a signal is a completion (D-80).
- The rule is checkable at review time and not by machine: nothing can tell a
  door that *cannot* wait from one that merely does not yet.

**Left owed.** The check. A door that can wait is a property of what it calls,
and this project has declined whole-program analysis before for good reasons
(D-147). The honest position is that this is a rule a reviewer applies, and
the entry exists so the reviewer knows to.

## D-151 A completion may carry a handle, and the receiver reserves its slot — Status: Accepted (2026-08-25)

**Context.** `011` closed Phase 5 owing this: *"no completion carries a handle,
so the ring cannot be the delegation path the channels are."* Phase 6 makes it
concrete, because `open` is the first operation whose whole purpose is
producing authority.

The question is who accounts for the handle-table slot.

**Options.**

1. **The kernel installs it** at completion time. Simplest: the operation
   finishes, the kernel puts the handle in the receiver's table, the completion
   names it.
2. **The receiver reserves it at submission**, from the same quota and by the
   same admission the completion slot already uses.
3. **Defer** — the ring stays data-only and every authority-producing operation
   detours through a channel.

**Decision.** Option 2. A submission that may produce a handle reserves a
handle-table slot alongside its completion slot, and is refused at admission if
either is unavailable.

**Rationale.** Option 1 reintroduces exactly the failure the reservation model
exists to delete. D-79 chose RIO-style admission over io_uring's `CQ_NODROP`
so that **an unreportable operation is impossible rather than unlikely**; a
handle installed at completion time can fail for a reason the submitter was
never told about, and the ring then has two flow-control models — one for the
completion, none for the authority it carries.

**The io_uring evidence, corrected.** An earlier draft of this entry said
io_uring grew "direct descriptors — `IORING_OP_ACCEPT_DIRECT`" *because*
completion-time installation exhausted the descriptor table. A research pass
checked it against the primary record and both halves were wrong.

There is **no `IORING_OP_ACCEPT_DIRECT` opcode.** It is `IORING_OP_ACCEPT`
with `sqe->file_index` set; `io_uring_prep_accept_direct()` is a liburing
helper, not an ABI entry. And the merge argument was never exhaustion: Begunkov's
5.15 cover letter, the `IORING_FILE_INDEX_ALLOC` commit message and
`io_uring_registered_files(7)` all give **per-operation overhead and
round-trips** — collapsing accept, register-update and close into one
operation. None of them mentions backpressure or table limits.

**The conclusion survives, on a better argument.** io_uring can install at
completion time only because it has an escape this project has explicitly
refused: `IORING_FEAT_NODROP` gives it an **unbounded kernel-side overflow
backlog** for completions that will not fit. D-79 rejected exactly that in
favour of RIO-style admission. Having refused the backlog, completion-time
installation has nowhere to put a failure — so reservation is not a
preference here, it is what the earlier decision already implies.

Two observable symptoms in io_uring's own source support the reading.
`io_accept_prep()` refuses multishot combined with a *specific* fixed slot,
because one submission producing unbounded installs is the case that forces
kernel-managed allocation. And `IORING_FILE_INDEX_ALLOC` still does not
reserve at submission: it allocates at completion and reports `-ENFILE` in
`cqe->res` when the range is full — a resource failure delivered at the one
moment the submitter can no longer do anything about it.

Zircon is the shape this entry chose, from the start: a channel read takes a
caller-supplied handle array, so the space is committed before the message
moves.

Option 3's cost is two mechanisms and two round trips for one idea, and it
does not avoid the decision — it moves it to whoever writes `open`.

**Consequences.**

- A submission carries what it may produce. The opcode says whether a handle
  can come back, and admission checks the handle quota then, not later.
- Refusal is backpressure and reads like it: the program is told at submit,
  which is the moment it can do something about it.
- The rights on a handle arriving by completion **attenuate the way D-110 makes
  the channel path attenuate**. Otherwise the ring becomes the widening path
  around that discipline, which is the one thing that would make it worse than
  the channels it is joining.
- Quota accounting moves earlier and has to be released if the operation is
  cancelled or its program retires (D-153).

**Left owed.** The reservation must be *returned* on every terminal path — a
refused operation, a cancelled one, a retired program's. That is three paths
and the third is D-153's, so this entry does not close it.

## D-152 Time is read from a page, and the page is global — Status: Accepted (2026-08-25)

**Context.** Phase 4 made the counter the clock (D-57). Reading it from ring 3
currently means a trap. Every OS that cares eventually publishes a read-only
page instead — Linux's vDSO `clock_gettime`, Zircon's vDSO — and this is one
step behind the work already done.

Two questions. Whether time is ambient at all, and if so, whether the page is
per-program or shared.

**Options for the second.** One shared read-only page, mapped into every
program. Or a page per program, which is what a future that wants to
*virtualise* time would need.

**Decision.** A read-only page, mapped into every program, **global**. It
carries the counter's zero, its frequency, and enough for a program to convert
a `rdtsc` into nanoseconds without trapping.

**Rationale.** `005` forbids the speculative API, and a per-program page is one:
nothing this OS has decided to want needs it. Virtualising time is a
container's problem, and this project has no containers and no plan for them;
adding the machinery now means a page per program, an accounting question, and
a second thing to keep in step, for a capability nobody has asked for.

**Time being ambient at all is the more interesting half, and it is deliberate
rather than incidental.** WASI makes clocks capabilities: a program that was
not given a clock cannot read one. Zircon treats monotonic time as ambient.
Both are defensible and this project should not arrive at either because a page
was easier than a door. The argument for ambient here: monotonic time carries
no authority — it names no object, grants no access, and a program that cannot
read it can still measure duration by spinning. What it would buy is
*determinism*, which is a replay and sandboxing property, and this OS has not
claimed it. Recorded so that a future that wants deterministic replay knows it
is giving up a page and not a principle.

**Consequences.**

- `sys_clock_get` stops being OWED and becomes a load. `libs/timebase` already
  has the arithmetic and it moves to ring 3 unchanged.
- The page is one more thing mapped into every program, and one more thing in
  the address-space layout — an entry in D-46's KASLR ledger.
- A wall clock is still absent. This publishes the monotonic counter;
  `CLOCK_REALTIME` needs an epoch and this kernel has never been told what time
  it is.

**Left owed.** The page's layout is a shared structure a program reads, so it
needs a version field for the reason the ring header has one (D-79), and the
same copy-out-then-validate discipline does *not* apply — the kernel is the
only writer. Both go in the charter.

## D-153 An operation in flight is collected with its program's zombie — Status: Accepted (2026-08-25)

**Context.** A program submits an operation and retires before it completes.
The completion has somewhere to go only while the program's ring exists, and
D-117 already makes a retired program's resources a zombie that somebody
collects later. What happens to the operation was unspecified, and it touches
D-117 directly: **a completion landing for an already-collected slot is the
shape of the bug.**

**Options.**

1. **Dropped at post.** The poster checks whether the program is still there
   and discards the completion if not.
2. **Refused at submit** once the program is retiring, so nothing is in flight
   when it goes.
3. **Collected with the zombie.** The ring outlives the program until whoever
   collects the zombie collects it too.

**Decision.** Option 3.

**Rationale.** It is the machinery that already exists. `LastRites`, `orphan`
and `collect` were built by D-117 for the program's *memory*; an operation in
flight is one more thing the zombie holds until a thread with nothing better to
do picks it up. `Bootstrap::ring` is already an `Arc` (D-126), so the ring
already outlives the moment the program stops running.

Option 1 needs the poster to know the slot's **generation** — not just that a
program is there, but that it is the same one — which is the check every other
use-after-free in this kernel has needed and the reason handles are
generation-tagged at all. Getting it right means reinventing that here; getting
it wrong means posting into a re-used slot.

Option 2 needs the submitter to lose a race it cannot see: retirement is
decided elsewhere, and "is it retiring" is true a moment after it is asked.

The cost is that reclamation is not complete until in-flight operations have
finished or been abandoned, so a zombie can outlive its program by an operation
rather than by a scheduling quantum. That is the honest cost of the thing being
asynchronous.

**Consequences.**

- `LastRites` grows what the ring was holding. `collect` releases the
  reservations D-151 took — handle-table slots included — which is the third
  terminal path that entry left owed.
- A completion posted to a zombie's ring is written and never read. That is
  correct: exactly one terminal completion is still posted for every operation
  (D-79), and nobody is waiting for it.
- The check is that frames, handles and reservations all come back after a
  program retires *with an operation outstanding* — which is a state the
  existing reclamation checks do not reach.

**Amended by `012` §10: exit must be a wait, not a signal.** Closing an
io_uring instance cancels pending requests automatically — and until March 2025
those cancellations happened *out of line*, so applications could not rely on
ring exit meaning the I/O had finished. The symptom in Axboe's cover letter was
**mount points staying busy after the test process had exited**. Six years of
io_uring did it the other way and it was a bug.

Three consequences for this entry, none of which it originally said:

- **Collection must wait for the operation, not merely mark it abandoned.**
- **The zombie must hold every resource any in-flight operation can still
  touch.** The busy-mount symptom is what happens when it does not, and it is
  the same failure as a completion posted into a re-used slot.
- **Do not put a grace-period-based refcount on the teardown path.** The same
  fix replaced a `percpu_ref` with a plain atomic because the RCU grace period
  was too slow there, and this kernel's reclamation is already deferred to
  whoever has nothing better to do (D-117).

**Left owed.** Cancellation is not this. An operation whose program retires is
abandoned; an operation a *living* program wants to stop needs the advisory
cancel D-79 describes, and nothing here builds it. `012` §8 adds what that will
need: two completions, order unspecified, and `-EALREADY` meaning "your
completion is coming, just not because of you".

## D-154 SMP stays at Phase 12; its seam is paid now — Status: Accepted (2026-08-25)

**Context.** Four entries carry debts that cannot be discharged on one CPU:
D-126 owes `LEAVES_HELD` a move into `arch::percpu`; D-129 owes the two-caller
race in `pic::silence`; D-141 and D-145 owe the interleavings in the handle
arena and `receive_blocking`, neither of which can be reproduced without a
second thread inside a window between two lock acquisitions.

Meanwhile the whole lock discipline — D-105's borrow token, D-121's no-closure
rule, D-126's leaf detector, D-70's rules — was **designed** for SMP and has
only ever been **exercised** by one CPU.

**Options.**

1. **Pull SMP forward, before storage.** Exercise the concurrency design at its
   smallest, so every later phase is written against a real second CPU.
2. **Pull it to just after Phase 6.** Storage first, so the machine is useful,
   then a second CPU before the network stack, where concurrency bites hardest.
3. **Leave it at Phase 12 and change nothing.**
4. **Leave it at Phase 12 and pay the seam now.**

**Decision.** Option 4. SMP stays where the roadmap put it. The **per-CPU
seam** is made real now: state that must become per-CPU moves into
`arch::percpu` while there is one CPU and the move is mechanical.

**Rationale.** The argument for pulling SMP forward is real — retrofitting it
is the classic disaster, and four debts wait on it. The argument against is
that this kernel has already done most of the expensive half: the lock ranks, the
leaf discipline and the borrow token all exist because D-56 and D-70 asked for
them in Phase 5.

**`arch::percpu` does not, and an earlier draft of this entry said it did.**
D-81 wrote "`arch::percpu` has to exist for ring 3 regardless" — and it did
not, because ring 3 went through an interrupt gate rather than `syscall`
(D-96), so `swapgs` was never needed and the module was never written. The
claim was corrected before this entry was an hour old, which is the same
defect class D-140 is about and is why that entry says a `SAFETY` comment
naming the wrong reason passes review exactly as well as one naming the right
one.

And an OS that can schedule on two CPUs and still cannot open a file is worse
than one that can open a file. Storage is what makes the machine a machine.

**What makes option 4 different from option 3** is that the debts stop
accumulating. Every static that will need to be per-CPU is cheaper to move
today than after the network stack has three more of them, and moving it is a
mechanical change with no behaviour to get wrong while there is one CPU. The
*races* stay unreproducible; the *structure* stops being owed.

So the seam is **built**, not merely moved into: `arch::percpu` becomes a
module holding one `PerCpu` block, with an accessor every caller goes through.
On one processor the accessor returns the single instance. When an AP starts,
the accessor reads `gs:` and **nothing else in the tree changes** — which is
the entire value, and the reason it is worth doing before there is a second
CPU rather than during.

**Consequences.**

- `arch::percpu` exists, holding `spinlocks_held`, `leaves_held` and the
  leaf-violation record. Every caller goes through it. D-126's owed item is
  discharged structurally.
- The `gs:`-based accessor is **not** built, because there is nothing to point
  it at. The seam is the module boundary, not the segment register.
- The three races remain recorded and unreproducible. Their entries say so, and
  none of them is reachable.
- Phase 12 inherits a smaller job: bring up an AP, and the state it needs is
  already where it needs to be.

**Left owed.** The honest test for an unreproducible interleaving is a stress
check with a flake budget, and this project has no flake budget and does not
want one. Whoever brings up the second CPU decides what that costs; it is named
in D-141 and D-145 and not decided here.

## D-155 Two checks whose inputs made the property invisible — Status: Accepted (2026-08-25)

**Context.** The last of the Phase 0–5 audit's Tier 3 findings that reach real
behaviour. Both are checks that exist, pass, and cannot fail — not because the
assertion is wrong, but because **every input they were given makes the right
answer and the wrong answer identical**.

**A framebuffer's span was only ever tested unpadded.** `FramebufferLayout`
takes a `width`, a `bytes_per_pixel` and a `pitch`, and `pitch` exists
precisely because rows can be padded. Every test used geometry where `pitch ==
width * bytes_per_pixel`, so the two are interchangeable in all of them, and
`row_bytes.checked_mul(height)` survives the suite. On a padded framebuffer —
which is most real hardware, and the only reason the field exists — that maps
short by `(pitch - row_bytes) * height` bytes and the bottom of the screen is
off the end of the mapping.

**A clock's rounding was only ever tested where rounding did not matter.**
`Timebase::new` computes a fixed-point multiplier with `+ denominator / 2`,
which is round-to-nearest and halves the worst-case error of every conversion
made through it. Change it to `+ denominator - 1` — round *up* — and every
test passes, including a docstring claiming that "losing a bit of the
multiplier breaks it".

**Decision.** Both are given inputs that distinguish the answers.

The framebuffer test uses `1024×768` at a pitch of 4352 against a row of 4096,
and asserts the span is the pitch's *and* is not the visible width's — the
second conjunct being what makes the first mean something.

The clock test checks the **claim** rather than the trick: for six real
frequencies it computes both integer neighbours of the exact ratio and their
distances, and requires the multiplier to be the closer one. That is what
"nearest" means, stated without reference to `+ denominator / 2`, so it catches
round-up, round-down and anything else.

**Rationale.** These are the same defect as D-142's `TYPICAL = 0xFEE0_0900`,
which sets both flag bits and so could not tell them apart. The pattern is
worth stating as a rule now that it has appeared three times:

**A test input that is realistic can be worse than one that is contrived.** A
square framebuffer with no padding is a real configuration and a useless test
value. A frequency whose ratio lands nowhere near a halfway point is a real
clock and proves nothing about rounding. The values a check needs are the ones
where the possible answers *differ*, and those often have to be constructed.

D-136's "pairwise distinct" instruction is the same idea from the other end,
and `005`'s testing section should carry both.

**Consequences.**

- Two host tests, both broken once with the exact mutation the audit named.
- No production code changed. Both computations were correct; neither could be
  shown to be.
- The audit's Tier 3 list is closed apart from `ps2-keys`, which anchors 21 of
  about 89 base scancodes and needs a transcribed table rather than a better
  input.

**Left owed.** `libs/ps2-keys`' scancode table. A swap of two unanchored keys
survives, and the fix is to transcribe set 2 from a source rather than to think
harder about the inputs — which is the D-142 treatment and a session's work, not
an hour's.

## D-156 The scancode table is transcribed, and one code is deliberately unanchored — Status: Accepted (2026-08-25)

**Context.** The last of the Phase 0–5 audit's Tier 3 findings. `libs/ps2-keys`'
set 2 table maps about eighty-nine make codes to keys, and twenty-one were
anchored by a test. A swap of two unanchored codes — `0x33`/`0x34` would
exchange **H and G on every keyboard** — survived the whole suite.

This is D-142's shape and needed D-142's treatment: not a better test input, a
**transcription from the document that fixes the values.**

**Decision.** Sixty-eight single-byte make codes are written out as literals
from Andries Brouwer's *Keyboard scancodes* §10, the set 2 table Linux's own
documentation points at, and asserted in both directions — every code names its
key, and no two codes name the same key, since a duplicate is a transcription
error the first loop cannot see.

**The sixty-ninth is left out, and that is the part worth recording.**

The source read as `\` = `0x5C`. This table has `0x5D`. Mainstream references
agree with the table; a second primary source could not be reached to settle
it, and one summarising fetch is not enough to overturn a value the kernel has
been running on.

Three options and only one is honest:

1. **Assert `0x5D`** — the table's own value. That is the definition restated,
   the exact defect this test exists to remove (D-124).
2. **Assert `0x5C`** — the one source. That is asserting a guess against
   working code, and would have turned the gate red on a value that is probably
   right.
3. **Leave it unanchored and say why.**

Option 3. **Sixty-eight anchored with the gap named beats sixty-nine with one
invented.** The comment states the disagreement, so the next person to reach a
second source knows exactly what to check and what it would settle.

**Rationale.** The transcription found the discrepancy, which is the argument
for doing it at all: a table checked against itself agrees with itself, and the
only way to learn that two documents disagree is to compare them. That the
disagreement is unresolved does not make the exercise a failure — it converted
an invisible uncertainty into a written one.

It also sets the rule for the next transcription, and there will be more:
**where the source and the code disagree and neither can be confirmed, anchor
neither and record the conflict.** A test is a claim, and this project's
standing position is that a claim nobody can check is worth less than no claim.

**Consequences.**

- Sixty-eight codes are pinned in both directions. Swapping `0x33`/`0x34`
  turns it red; it did not before.
- One code is documented as unverified, in the test that would otherwise be
  assumed to cover it.
- The audit's Tier 3 list is closed.

**Left owed.** `\`'s make code, against a second primary source — the IBM PS/2
Technical Reference or a keyboard controller datasheet. Ten minutes for anyone
with either to hand, and worth doing before a keyboard layout depends on it.

Extended codes — those behind an `E0` prefix — are not transcribed here. They
are the arrow keys, the navigation cluster and the keypad, and the same
treatment applies to them.

## D-157 A check that quantifies must read its domain, not restate it — Status: Accepted (2026-08-25)

**Context.** A battery run over the finished Phases 0–5 — the gate, TLC, and
all 29 mutations — passed on the first attempt. What it turned up was not in
the kernel but in two of the checks watching it, and both had the same shape:
**a test named for a universal claim whose body iterates a list copied from
the thing it is checking.**

`every_invariant_the_model_defines_is_checked` was written for D-148, whose
whole point is that TLC checks the names in `INVARIANTS` and nothing else — so
an empty block reports "No error has been found" and `cargo xtask model-check`
passes having checked nothing. The test guarded that by looping over five
invariant names written out in an array. It could not see a sixth. A property
added to `Ring.tla` and forgotten in `Ring.cfg` would be unchecked by TLC, and
the test whose name promises otherwise would stay green.

`every_task_the_parser_accepts_is_named_in_the_help` had it worse: seventeen
task names in an array, checked against a parser whose `match` holds the same
seventeen. Two copies of one list, and the test passes on the day they
disagree — because the copy it reads is the copy it was written from.

**Decision.** Both derive the domain from the source of truth.

The model test reads `Ring.tla`, takes every top-level definition, subtracts
the specification's own machinery — `vars`, `Init`, `Next`, `Spec`, and every
action `Next` disjoins, *itself parsed out of `Next`* — and requires whatever
remains to be named by the config. It reads the config's block in the other
direction too, so a name checked but not defined is caught as well, and it
refuses an empty block outright.

The CLI test reads its own source through `include_str!("cli.rs")` and takes
the task names out of the parser's match arms.

**Rationale.** A list restated beside its source is not evidence about the
source. It is a second copy with a test-shaped wrapper, and the failure it
cannot see is exactly the failure it was written to catch: the two drifting
apart. Both of these were sound when written and had already stopped being
sound — the model gained no sixth invariant, but nothing was stopping one.

The rule generalises past these two, which is why it is here rather than in a
commit message: **if a check's name begins "every", "all" or "no", its body
must enumerate the domain rather than name its members.** `vendor.rs` is the
worked example of the right shape — it walks `boot/` and `tools/` for
`CHECKSUMS` files, cross-checks both directions, and asserts the counts match,
so a new vendored directory that nothing verifies is a red test rather than a
silent gap. `tasks.rs` derives from `PASSES`; `qemu.rs` parses the sysdep file
and pins the count at sixteen. The two that were wrong were the two that had a
list.

This is D-124's "write the check from the claim, not the implementation" one
level up. There, restating the code passes trivially; here, restating the
*data* passes trivially. The tell is the same in both: the check contains a
copy of its subject.

**Consequences.**

- Two host tests rewritten to parse their sources. Five deliberate breaks, each
  confirmed red: an invariant the config misses, a config name the model does
  not define, an empty `INVARIANTS` block, an unignored file in `models/`, and
  a parser arm with no help entry.
- A third check added — `models/` holds the model and nothing that is not
  ignored — because the same run found five of TLC's binary counterexample
  traces committed to the repository. `.gitignore` covered
  `models/*_TTrace_*.tla` and not the `.bin` written beside it, and nothing
  looked at that directory. The files are untracked, the rule is extended, and
  the check is filesystem-based rather than `git ls-files` because the gate
  does not shell out to git anywhere and should not start here.
- A sweep of the other fifty-odd universally-named checks found no third
  instance. `tasks.rs`, `qemu.rs`, `vendor.rs` and `paths.rs` all enumerate.
- `005`'s testing section carries the rule.

## D-158 One ELF reader, and the verifier uses the loader's — Status: Accepted (2026-08-25)

**Context.** D-105 named it and left it: `xtask/src/elf.rs` carried its own
transcription of the ELF magic, the class byte, the endianness byte, `PT_LOAD`,
`PF_W`, `PF_X` and every file- and program-header offset, while `libs/elf64` —
the crate the kernel actually loads programs with — carried the same table
again. Two readers of one format in one repository, with nothing comparing
them.

The cost was not hypothetical and had already been paid twice. D-124 added
`e_machine` and the program-header stride checks to `elf64` and found `xtask`'s
copy *two validations short*, so the build verifier would have shipped an
aarch64 kernel image. A test named
`the_two_refusals_libs_elf64_has_are_here_too` existed for no reason except to
hand-maintain the sync — a test whose name concedes the defect.

**Options.** Leave it and keep the sync test. Move the section reader into
`elf64` so one crate reads everything. Or have `xtask` read the header and the
program headers through `elf64` and keep only the section table.

**Decision.** The third. `xtask::elf::parse` is now
`elf64::Elf::parse(bytes)` plus a section walk; `parse_segments` and every
transcribed constant are deleted. The section table stays in `xtask` because a
*loader* never reads one and nothing else in the tree wants it — moving it into
`elf64` would grow the crate the kernel links for the benefit of one host
caller.

Two accessors were added to `elf64` to make it a complete substitute:
`SegmentKind::bits`, the inverse of `from_bits`, and `Segment::flags`, the raw
field a verifier needs in order to report what it objected to. Neither is
speculative — both have their caller in the same commit (D-41).

**Rationale.** The rule this settles is the same one D-157 stated a level up.
There, a check restated the data it was checking; here, a reader restated the
format another reader already knew. Both pass on the day the two copies
disagree, because the copy being read is the copy the author was looking at.
`x86-tables` and `trap-cause` disagreeing about vector 28 is the same story
with a third table.

The merge also makes `xtask` *stricter* for free, which is the tell that it was
behind: `elf64` takes the whole 64-byte file header before it looks at the
class byte, checks that the program-header table lies inside the image without
overflowing, and refuses `p_phoff` near `u64::MAX` — none of which the
verifier's own parser did.

**Consequences.**

- `xtask/src/elf.rs` loses seven constants, `parse_segments`, and every header
  offset except the section table's. `libs/elf64` gains two accessors and a
  round-trip test over the `p_type` table, exhaustive across the named range
  and its edges, which also proves no two named kinds encode alike.
- Every existing ELF test passes unchanged, which is the evidence the two
  readers did agree — and `elf64`'s error messages already contained the
  strings the verifier's tests assert on.
- Two tests were reframed rather than deleted. The sync test is renamed for
  what it now proves — an image for another machine or of another shape is
  refused — and still asserts the two constants, because a typo in `elf64`
  would move both readers together and neither would notice.
- `libs/elf64`'s module doc said the merge had not happened. It now says it
  has, and says what deliberately stayed behind.

## D-159 The lints that were measured and not adopted — Status: Accepted (2026-08-25)

**Context.** `005` says **measure what a tool does; do not reason about it**.
The workspace denies three clippy lints beyond the default set —
`undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`,
`missing_safety_doc` — and nothing recorded why the rest were not on. "It would
be noisy" is a guess, and the rule says to go and look.

**Decision.** Measured across the whole tree, host and kernel targets. None is
adopted, and this entry exists so the measurement is not repeated.

| Lint group | Hits | What they were |
|---|---|---|
| `clippy::pedantic` | 59 in six `libs/` crates | 17 cast, 7 `must_use_candidate`, 4 `doc_markdown`, the rest style |
| `clippy::nursery` | 4 | one real `redundant_clone`, in a test |
| `clippy::indexing_slicing` | 118 in `kernel` + `wost` + `libs` | every one behind a `get`/`take` that had already proved the length |
| the cast family | 66 in `kernel` + `wost` | sign-extension in `x86-paging` is the canonical-address arithmetic itself |

**Rationale.** Not one of them is a defect. The indexing count is the
interesting one: 118 sites, and every one sampled indexes a slice the function
has just bounds-checked — `elf64` reads `field[0]` only after `take(bytes, at,
2)?`, `acpi-tables` reads `head[15]` only after `bytes.get(..V1_LEN)`. The lint
cannot see the proof, so adopting it would mean 118 suppressions asserting that
a proof exists somewhere above. That is worse than nothing: `005` already
records that a suppression which reads as checked and is not falsifiable is
worse than the `allow` it replaced.

The cast lints are the same shape. `((address as i64) << 16 >> 16) as u64` *is*
`is_canonical`; the sign loss the lint reports is the mechanism.

**Consequences.**

- No lint added, no suppression added, no code changed by this entry.
- The one genuine finding — a `redundant_clone` in an `acpi-tables` test — is
  not worth a lint that costs 118 suppressions, and is left.
- If a future phase parses a new hostile format, the measurement is cheap to
  redo and this table says what the baseline was.
- What *is* worth keeping is the shape the measurement confirmed: the tree has
  no `.unwrap()` in `kernel` or `wost` outside the self-test, and its sixteen
  panicking calls are all deliberate halts on a broken kernel invariant — a
  refused yield, a resumed dead thread, a second boot handoff.

## D-160 A page is 4096 bytes in five places, and now something checks that — Status: Accepted (2026-08-25)

**Context.** Having merged the two ELF readers (D-158), the obvious next
question is how many other tables this tree states twice. Sweeping every
`const NAME: type = literal` across `libs/`, `wost` and `kernel` for a name
defined more than once turned up one answer that matters and two that do not.

**The page size is stated five times.** `frame-space` and `slab-layout` call it
`FRAME_SIZE`, `stack-layout` and `x86-paging` call it `PAGE_SIZE`, and
`kernel/src/mem.rs` has its own `FRAME_SIZE`. Four crates and a kernel module
with no dependency between them, each declaring 4096, and nothing comparing
them.

This is the most load-bearing constant in the kernel. Every layout is computed
from one of these: the frame allocator hands out frames, the slab allocator
divides them, the stack allocator strides between guard pages, and the page
tables map them. A frame allocator handing out 4096-byte frames to a slab
allocator that believes they are 2048 does not fail a test — it fails at the
second slab, in a way that looks like heap corruption.

**Options.** Make one crate own it and have the rest depend on that crate;
leave it and rely on the number being obvious; or discover the sites and
compare them.

**Decision.** The third. `xtask/src/machine.rs` scans `libs/`, `wost/src` and
`kernel/src` for every definition of a page-size constant and requires them all
to be the same value, and to be 4096.

Not the first option, because the dependency it would add is the wrong shape:
`stack-layout` and `slab-layout` are pure arithmetic parameterised by a size,
and making them depend on `x86-paging` to learn what a page is would put an
architecture crate under two that have no architecture in them. Their
independence is a property worth keeping; what was missing was not a dependency
but a comparison.

**The sites are discovered, not listed** (D-157). What the module names is the
*fact* — which constant names mean "the size of a page" — and it then goes and
finds every definition. A sixth transcription is covered the day it is written,
without touching the check.

**Rationale.** Three details make the check able to fail rather than merely
able to pass:

- **It refuses to find nothing.** Fewer than five definitions is a failure, not
  a pass, because a scan that stops matching reports success with nothing
  compared — the same failure shape D-157 found in the model's `INVARIANTS`
  block.
- **It refuses a computed value.** `PAGE_SIZE = 1 << 12` is not a literal this
  can evaluate, and a page size it cannot evaluate is a page size it cannot
  compare. Failing is honest; skipping would be a hole shaped exactly like the
  thing being checked.
- **It asserts 4096, not merely agreement.** Five files agreeing is also what
  five files being wrong together looks like.

**Consequences.**

- `xtask/src/machine.rs`, three host tests, broken twice: one crate told a page
  is 2048 (red, naming both sites), and the scan made to match nothing (red).
- One real duplication fixed: `CR4_OSXSAVE` was a function-local constant in
  *two functions of one file* — the one that sets the bit and the one that
  reads it back — with the same value and the same one-line doc. Hoisted.
- One duplication deliberately left. `DELIVERY_FIXED = 0` appears twice in
  `libs/x86-apic`, in `command` and in `local_vector`, and they are not one
  fact: the ICR's delivery-mode field and an LVT entry's delivery-mode field
  are different fields of different registers that both encode "fixed" as zero.
  Merging them would name two facts once. The doc comments differ because the
  facts do.
- `INTERRUPT_FLAG = 1 << 9` also appears twice, in `kernel/src/thread.rs` and
  in the self-test. That one stays on purpose and is the general rule: **a
  check does not import the constant it is checking.** The self-test asking
  about bit 9 independently is what makes it a check rather than a restatement.

## D-161 The list a test sweeps is itself a list that can fall behind — Status: Accepted (2026-08-25)

**Context.** D-157 said a check whose name quantifies must read its domain
rather than restate it, and claimed a sweep of the other fifty-odd
universally-named checks found no third instance. That sweep looked at test
*names* against their bodies. It missed the case where the body does iterate a
list — and the list is the thing that has fallen behind.

`libs/x86-apic` keeps `const ALL: &[(&str, Register)]` in its test module,
carrying a comment that reads *"Every register named here, so the sweeps below
cannot miss one."* Three tests iterate it: the alignment check, the offset
collision check, and — separately — a second hand-written list anchors each
register to the MSR the SDM gives it. Nothing tied any of it to the fourteen
`pub const NAME: Register` the crate actually defines. A fifteenth register
added and left out of `ALL` would shrink all three sweeps at once, and all
three would go on reporting success.

`libs/x86-apic::timer` has the same shape over the eight `Divide` variants.

**Decision.** Both lists are anchored to what they claim to enumerate, by the
means each one admits.

`ALL` for registers is compared against the crate's own source, read through
`include_str!("register.rs")`, **by offset rather than by count** — a list that
dropped one register and repeated another has the right length, and only the
offsets say so. The MSR anchoring list is then tied to `ALL`'s length, so a
register defined without a manual value behind it fails rather than passes
unmentioned.

`ALL` for `Divide` is anchored by an **exhaustive match**, which makes the
compiler the checker: a ninth variant is `error[E0004]: non-exhaustive
patterns` and the crate stops building. The length assertion sits after it for
the case the match is updated and the list is not.

**Rationale.** These are the two mechanisms available, and which one applies is
a property of what is being enumerated, not a preference. Enum variants can be
enumerated by the compiler, and where they can, that is strictly better than a
test: it fails at build time, in every configuration, without being run. A set
of `pub const` items cannot be, so the source is read instead — the same
technique D-157 used on `Ring.tla` and on `cli.rs`'s own match arms.

The general statement, which now covers three findings in one day: **a check is
only as good as the enumeration it sweeps, and an enumeration written by hand
beside its subject is a copy.** D-157 caught copies of a config and a parser;
this catches copies of a crate's own item list. The tell is identical in all
three — the check contains a copy of its subject.

**Consequences.**

- Two new tests in `libs/x86-apic`, four breaks, each red: a register defined
  and left out of `ALL`; a register dropped from `ALL`; a register with no MSR
  anchored to the manual; a `Divide` variant missing from `ALL`. The
  compile-time anchor was demonstrated separately — a ninth variant fails the
  build with `non-exhaustive patterns`, in the crate's production code as well
  as the test.
- The comment claiming `ALL` was complete is now a check that it is.
- `libs/timebase`'s `PLAUSIBLE_HZ` was examined and left. It is a list of real
  clock frequencies to test *against*, not an enumeration of anything the crate
  defines, so there is nothing for it to fall behind.

## D-162 The last unanchored scancode was a misread column — Status: Accepted (2026-08-25)

**Context.** D-156 transcribed 68 of about 89 single-byte set 2 make codes and
stopped one short. Backslash read as `0x5C` in the source and the table had
`0x5D`; mainstream references sided with the table, a second primary source
could not be reached that day, and the entry was left out with the reason
written down — *"one code unanchored and said so beats sixty-nine with one
invented."*

**Decision.** It is settled and anchored. There was no conflict.

Brouwer's correspondence table gives the backslash key three codes, one per
set: **`0x2B` in set 1, `0x5D` in set 2, `0x5C` in set 3.** Vetra's
translation table says the same independently — set 2 `5D/F0 5D`, set 3
`5C/F0 5C or 53/F0 53`. The `0x5C` that looked like a contradiction is the
**set 3** value, read out of the wrong column of a table that carries all
three.

This kernel puts the keyboard in set 2 and never in set 3, so `0x5D` is the
code it will see. The table's value was right the whole time; what was missing
was the reason to believe it.

**Rationale.** Worth an entry of its own for what the shape of the mistake
teaches. The source was correct, the table was correct, and the reading was
wrong — which is not a case that "check your sources" catches, because the
source *was* checked. What caught it was going back with a specific question:
not "what is backslash" but "what is backslash **in set 2**, and what is at
`0x5C` if not this". A table with three columns answers a question that names
its column and misleads one that does not.

D-156's refusal to guess was right and is what made this cheap. An asserted
`0x5C` would have been a green test enshrining the set-3 value, and the failure
would have shown up as one key on real hardware — which is exactly the class
D-156 existed to remove.

**Consequences.**

- `libs/ps2-keys`' set 2 table is anchored at **69 codes**, up from 68, and
  nothing in it is now unanchored on purpose. Broken once: the table given the
  set-3 code goes red naming the code and the key.
- Two independent sources are cited in the test's doc comment rather than one,
  and the set the value belongs to is named — which is the detail whose absence
  caused this.
- D-156's "left owed" item is closed.

## D-163 The handle table works through a shared reference, and a promise is a value — Status: Accepted (2026-08-25)

**Context.** Phase 6 slice 1 owes D-151: a completion must be able to carry a
handle. Two things stood in the way, and the second was not obvious until the
first was fixed.

**A program's table could not be written to after it was spawned.** `spawn`
built a `HandleTable`, inserted the bootstrap channel and wrapped it in an
`Arc`, and every mutating method took `&mut self` — so from that moment the
table was frozen. The ring is served from the syscall handler, where no
`&mut` to it exists and none can, because the table is reached through a
`BOOTSTRAPS` guard that must be dropped before anything else is taken (D-121).

**Wrapping it in a lock is the obvious answer and the wrong one.** `insert`
takes the handle arena's leaf guard. A table lock taken as a leaf and held
across that is leaf-under-leaf, which is what `lock_leaf` forbids and what the
machine check watches for; a token-bearing lock cannot be taken in trap context
at all, because `ThreadContext::claim` returns `None` inside a handler (D-55).
There is no lock that fits.

**Decision.** The count becomes an atomic and every method takes `&self`.

`id`, `mixer` and `quota` were already immutable after construction. The only
mutable state was `live: usize`, and it becomes one `AtomicU64` holding the two
counts `libs/handle-space`'s `Quota` packs — handles held, and slots promised.
No lock is added, no lock order changes, and `Arc<HandleTable>` becomes usable
exactly where it already was.

**A reservation is a value with a destructor, not a rule.** `reserve_slot`
returns a `SlotReservation` holding an `Arc` to the table. `install` consumes
it and keeps it; every other end — a refusal, a cancellation, a `?` on the way
through, the death of the program holding it — runs `Drop`, which gives the
promise back.

**Rationale.** D-151 asks that "the reservation is returned on every terminal
path", and names three paths. The trouble with a property stated as a list of
paths is that the list is a copy of the control flow, and it stops being a copy
the moment a path is added — which is D-157's rule one level down. Making it a
destructor obligation means the paths do not have to be enumerated in order to
be covered: there is no way to lose one, including down a path nobody has
written yet. Slice 3's dead-program path is the third in D-151's list and needs
no new code here to be correct.

The atomic is also what makes the two counts one word. Moving a promise to a
handle is a single compare-and-exchange, so no reader ever sees a table that
holds neither — which is what a quota check would otherwise be racing.

**Consequences.**

- `libs/handle-space` gains `Quota`: `reserve`, `commit`, `release`, `take`,
  `close`, all total, all host-tested, five deliberate breaks. The overflow
  test found a real edge on the first run — `reserve` could carry a full
  promise half into the held half — and the transitions are now guarded on the
  half being incremented as well as on the limit.
- `insert` splits: `fill` does the arena work and touches no count, so the two
  callers that charge differently share the part that does not.
- **One existing mutation had to be disambiguated.** `install`'s attenuation is
  the same line as `accept`'s, so `xtask/src/mutate.rs`'s one-site rule started
  failing — correctly, and immediately. The pattern now carries its next line.
- `live()` counts held handles and not promises. A check asking what a program
  can *reach* must not count what it has been promised, and the two are
  separately readable for exactly that reason.

## D-164 A wait that is a wait — Status: Accepted (2026-08-25)

**Context.** `014` opens by saying this is "the phase that gives the kernel
something to wait for", and D-149 puts the ABI first precisely so the waiting
shape is fixed before a filesystem depends on it. Slice 1 owes the first
operation whose honest answer is *not yet*.

Everything this kernel served through the ring until now answered immediately.
`perform` was a pure function of the submission: echo returns its argument,
uptime reads the clock. There was no in-flight state, and `Queue`'s wait queue
had been built in Phase 5 and never used.

**The obstacle is that a ring-3 thread had no way to stop running.** The
syscall arrives through an interrupt gate (D-96) and is served in trap context,
where `sleep_ns` cannot be used: it takes a `ThreadContext`, and there is none
inside a handler (D-55). A "wait" that could not block would be a spin, and a
program spinning is a program holding a CPU it is not using.

**Options.** Return "nothing yet" and let the program spin; run the ring from a
kernel worker thread that can block; or give the trap path its own way to yield.

**Decision.** All three of the pieces, each in its own place.

**An operation can be deferred.** `Opcode::Timer` is admitted, recorded in a
small pending table, and *not* completed. Its completion slot was reserved at
admission and stays reserved, which is what makes the later completion certain
rather than hopeful — the reservation is the difference between deferring an
operation and dropping one.

**A thread can sleep from a trap.** `thread::sleep_from_trap` is
`retire_from_trap`'s shape without the retirement: mark the caller `Sleeping`
with its deadline, hand the CPU on, and switch **through the frame**, which is
the mechanism preemption already uses and the only place a switch is legal
(D-54). The thread resumes inside its own handler and returns to ring 3 from
there.

**`RingWait` is the call that uses both.** Nothing in flight returns zero at
once — blocking on nothing is a hang. Something already due completes without
sleeping. Otherwise the thread sleeps to the soonest deadline and completes
what it finds on waking.

Not the worker thread. `012` §8 is emphatic that io_uring's history is a march
*away* from its pool, that its bounded pool alone would be `4 × nr_cpus`, and
that this kernel has sixteen thread slots in total. A state machine driven from
the waiting thread needs zero — the direction Linux was travelling and could
not finish because it had to keep every existing filesystem working. This
kernel has none yet, which is the one moment the cheap answer is also the right
one.

**Rationale.** The completion is posted by the kernel *after* the deadline, on
the waiting thread's behalf, so a woken program finds the entry already in its
queue. That is what makes it a wait rather than a poll dressed up, and it is
the shape slice 5's polled NVMe read reuses without changing the ABI.

`Queue`'s `WaitQueue` is still not what a ring-3 waiter uses, and this entry
says so rather than leaving it looking like an oversight: `WaitQueue::wait`
takes a `ThreadContext`, so it is the mechanism for a *kernel* thread waiting
on a ring. A ring-3 thread waits through the scheduler, because that is where a
trap-context switch is legal.

**Cancellation is an operation, not a substitute.** `Opcode::Cancel` has its
own reservation and its own completion. The operation it ends still gets
exactly one terminal completion, carrying `STATUS_CANCELLED` rather than a
result. `012` §8 names the double-completion class behind two io_uring CVEs,
and this is the shape that has no room for it. A cancellation naming nothing is
`STATUS_NOT_FOUND` and **not an error** — io_uring's `-EALREADY` — because an
operation that completed between the submission of the cancel and its service
is a race the program cannot avoid.

**Consequences.**

- Three opcodes: `Duplicate`, `Timer`, `Cancel`. Seven statuses where there
  were two.
- Six machine checks, seven deliberate breaks, four mutations.
- `sleep_from_trap` returns whether it actually yielded. A machine with no
  other thread to run, or a clock that never started, gets an honest zero
  rather than a hang — a wait nobody can end is worse than no wait.
- The pending table holds one entry per submission slot, which is not
  arbitrary: an in-flight operation occupies a submission's worth of admission
  until it completes, so a larger table could never be filled.
- `Pending` is a timer and nothing more. D-41 forbids the abstraction with one
  implementation; the second kind arrives with the block device in slice 5, and
  that is when it becomes worth generalising.

## D-165 A check that could not fail, found by breaking it — Status: Accepted (2026-08-25)

**Context.** Slice 1's most important property is D-151's: **a promised handle
slot is given back on every terminal path.** A leak there is not a crash. It is
a table that shrinks by one every time a program asks for something it cannot
have, and it surfaces hours later as a process that cannot open anything.

The check written for it set up a table with room for exactly one handle, put
the source handle in it, and submitted three operations that were refused. It
asserted afterwards that nothing was still promised, and it passed.

**It could not have failed.** With the table already full, all three refusals
happened *at the reservation* — `reserve_slot` returned `QuotaExhausted` and no
promise was ever made. `promised() == 0` was true because nothing had been
promised. Deleting the destructor whose entire job is returning promises left
the check green.

**How it was found.** By D-28, and only by D-28. The break was applied, the
boot went red — and it went red on a *different* check, which is what gave it
away. `a completion cannot widen what was offered` also asserts `promised() ==
0`, and its table has room, so its refusal happens after a promise is taken.
The check that was *about* the property could not see it; a check that
mentioned it in passing could.

**Decision.** The check is rebuilt so that every submission reserves and then
fails, at a different point each time: at the lookup (a value naming nothing of
this table's), at the right (a handle held without `DUPLICATE`), and at the
attenuation (asking for more than is held). The table is deliberately roomy,
which is the opposite of the instinct that produced the first version.

The full-table case is not deleted — it is a different claim and gets its own
check. *A full table refuses at submission* is about the moment the program is
told, which D-151 argues is the whole point of reserving at all. Merging the
two is how the first one came to be unfailable: it was trying to be about both.

**Rationale.** This is D-155's rule — choose inputs where the answers differ —
arriving from a direction that rule did not cover. There the inputs made two
*answers* indistinguishable. Here the input made the code path under test
**unreachable**: the check never ran the lines it was written about. That is
worse, because no amount of staring at the assertion reveals it. The assertion
is correct.

The general form, and it is the third thing this rule has cost:

> **A check must reach the code it is about.** Set-up chosen to make the failure
> easy to trigger can make it impossible to reach — a full table refuses early,
> a closed channel refuses early, an unmapped page faults early. If the refusal
> under test is the *second* thing that could go wrong, the input has to get
> past the first.

`005`'s testing section carries it.

**Consequences.**

- One check rebuilt, one check added, and the break that found it is now a
  mutation, so it cannot rot back.
- Two checks now assert `promised() == 0` from different set-ups, which is the
  redundancy that caught this and is worth keeping.
- Nothing in the kernel changed. The destructor was correct; the check could
  not show it.

## D-166 The time is a page, not a call — Status: Accepted (2026-08-25)

**Context.** D-152 settled the shape of a published clock and D-149 put it in
this phase, before a filesystem exists to want it, because a call that becomes
a load later is two doors to one operation for ever. `sys_clock_get` was OWED
over `libs/timebase` (D-114): the mechanism was complete and the door was
missing.

**Decision.** One page, mapped read-only into every program at a fixed address,
with two regions.

**The constants region has no sequence counter and needs none.** Version, page
size, counter rate — written once, before the page can be mapped anywhere, and
never again. Zircon's arrangement, and it removes the most error-prone reads
from the protocol entirely.

**The clock region carries Linux's seqlock**, both barriers written out even
though on x86 the store-store one is a compiler barrier and no instruction —
because an aarch64 port should not have to rediscover that it is not free
everywhere.

**This clock does not need a periodic update, and that is not a shortcut.**
Linux rewrites its published clock every tick because its clocksource may be an
HPET, or a TSC that stops in idle, and because NTP adjusts the multiplier
continuously. This kernel refuses to calibrate at all unless the counter is
invariant, so the conversion is fixed and the origin never moves: a reader
computes `convert(rdtsc - origin)`, which is *exactly* what `time::now_ns`
computes. The seqlock is still here and still correct, because the page is
republished on the two transitions that do exist — calibration, and revocation.

**The base is split, and the obvious form is wrong.** `base_ns << shift`
overflows a `u64` after about eight seconds of uptime at a 1 GHz counter's
shift, so "store the nanoseconds pre-shifted" cannot be taken literally. What
is stored is whole nanoseconds beside a carried *fraction* of one, always below
`1 << shift`. Linux splits it the same way. The fraction is not decoration: a
clock updated at a kilohertz that dropped it would fall a microsecond behind
every second.

**The revocation gate is why the syscall stays.** `mode` is the reader's
permission to use the page at all, and D-152 requires the fallback to be always
reachable — so the ring's `Uptime` opcode is not decoration either, and the
check that revokes the clock asks the ring for the time on the way past.

**Rationale, and the three things that were wrong first.**

*The page exists even when the clock does not.* The ABI says there is a page at
`WIZ_TIME_ADDRESS`, and a program that reads it and takes a page fault has been
lied to. So the page is built at boot whatever the clock did, `mode` is
`MODE_NONE` on a machine that never calibrated, and a spawn that cannot show it
is refused rather than quietly given a space with a hole in it.

*The preemption precondition was measured in the wrong place.* D-152 asks that
"the writer is not preemptible" be established rather than assumed, and the
first counter recorded **calls** made with the interrupt flag set. The check
built on it failed immediately — correctly, because a kernel thread calling
`publish` with interrupts on is fine: the guard clears the flag before the
counter goes odd and restores it after it goes even. What matters is the
*window*, so that is what is counted now.

*The odd marker has no witness on one CPU.* Deleting the write that marks the
window open left every check on this page green. Nothing else runs between the
two increments — the guard clears the interrupt flag — so there is no observer.
This is the class D-154 handles by building the seam and checking what can be
checked: the writer now reads its own marker back and counts a failure, so a
writer that stops announcing itself is caught by the only observer that exists.
A second CPU would give a better one.

**Consequences.**

- `libs/timebase` gains `resume`, `advance`, `from_parts` and `Seq`, all
  host-tested, four deliberate breaks. The drift test needed two attempts: the
  first chose a step of 3,192,000 ticks at 3.192 GHz, which is exactly a
  millisecond, so the conversion had no fraction to lose and dropping the carry
  left it green — D-155's rule, hit again by the person who wrote it down.
- Six machine checks, six breaks, all red. The page-and-call check is a
  **sandwich rather than a tolerance**: the page's reading is taken between two
  calls and must lie between them, so a multiplier or shift off by one fails
  immediately and no slack hides it.
- `sys_clock_get` moves from OWED to MAPS and is now a load. The sysdep tally
  is 5/10/1 where it was 4/11/1.
- `WIZ_CALL_CLOCK_GET` is **deleted** from the ABI, and `WIZ_CALL_RING_WAIT`
  takes 3. The owed calls renumber, which costs nothing because none is
  implemented and would cost an ABI break later — the same reasoning D-118 used
  the last time they moved.
- The C header gains the time page's address, version, modes, offsets and
  layout, and the cross-check compares **every one of them** against
  `timepage.rs`, including each field offset. A `mode` read from the wrong
  place is a clock that reads as revoked, or worse as usable when it is not.
- One more entry on D-46's KASLR ledger: `TIME` at `0x0000_1000_0000_0000`.
  It costs nothing while no two programs share a table (D-113), and it is what
  makes the address a `#define` rather than a lookup.

## D-167 A dead program's work is finished, not forgotten — Status: Accepted (2026-08-25)

**Context.** D-153 said an operation in flight becomes one more thing the
zombie holds, and `012` §10 added the part the original entry did not say:
**exit must be a wait, not a signal.** io_uring cancelled out of line for six
years and the symptom was mount points staying busy after the process had
exited.

Until slice 1 there was nothing to say this about. Every operation this kernel
served completed inside the call that submitted it, so a retiring program had
no work in flight by construction. `Opcode::Timer` changed that, and D-153's
debt became reachable in the same commit that made it possible.

**The hazard is specific.** A retired program's ring pages go back to the
allocator in `reclaim`. An operation still in flight is one that will later
write a completion, and the page it would write into is about to be somebody
else's. That is not a leak; it is a write to freed memory, arriving at whatever
time the operation happens to finish.

**Decision.** The ring is quietened before anything it could touch is released.

`Queue::abandon` ends every operation in flight and **posts the completion it
was owed**, into pages the zombie still holds. When it returns, nothing is left
that could reach them, and `reclaim` drops the ring immediately after.

Not "marked abandoned and left". The completions go into a ring nobody will
read, and that is not waste: posting them is what *spends the reservations*.
A reservation never spent is a ring whose accounting says work is outstanding
for ever, which is invisible until something asks — and slice 5's device will
ask.

**No grace period and no refcount.** `012` §10 names the third consequence and
it is the easiest to violate by accident: a teardown that waits for a count to
reach zero "eventually" is exactly the out-of-line cancellation that took
io_uring six years to undo. Every operation this kernel has today ends
synchronously, so one sweep is the whole wait. An operation that could not — a
read the device has already been told to do — is slice 5's, and the shape it
must fit is this one: the sweep ends when `is_quiet` is true and not before.

**The handle promise needs no code at all**, which is the part worth recording.
D-151's third terminal path is "a retired program's", and D-163 made the
reservation a value holding an `Arc` to its table. So a program that dies
holding a promise leaves the table alive exactly as long as the promise does,
and the promise's destructor gives the slot back and lets the table go. The
teardown drops its reference and the table simply does not leave. "The zombie
holds every resource any in-flight operation can still touch" is true by
construction rather than by remembering, which is what D-163 predicted and this
slice is the confirmation of.

**Rationale.** Three checks, and each is a different claim.

*Work in flight is ended before the ring is released* asserts the completions
arrive, say `CANCELLED` rather than pretending to be results, and — the part
that matters — that `outstanding` returns to zero. It also sweeps twice, so
teardown is idempotent: `reclaim` is not the only path that can reach a zombie.

*A promise outlives the program that made it* observes the table through a
`Weak`, which is the only way to ask "is it still there" without being the
reason it is, and then **uses** it: the promise is spent against the orphaned
table, because surviving and working are different claims. Then the `Weak` must
stop upgrading, because a table nothing will ever free is the other failure.

**Consequences.**

- `Queue::abandon` and `Queue::is_quiet`; `Reclaimed` gains `abandoned`.
- Two machine checks, three breaks, all red. Breaking the promise's `Arc` —
  giving the reservation a table of its own — reddens four checks at once,
  which is the redundancy D-165 argued for doing its job.
- The ordering in `reclaim` is now load-bearing and says so: quieten, then drop
  the table, then drop the ring, then release the space.
- Slice 5 inherits the shape rather than the code. A polled NVMe read cannot be
  ended by writing a completion — the device has the command — so `abandon`
  will have to wait for the controller there. What it must not do is return
  before `is_quiet`.

## D-168 A driver claims a device, not a port range — Status: Accepted (2026-08-25)

**Context.** D-77 closed half an authority debt and named the other half. A
port claim proves **exclusivity** — nobody else drives these ports — and says
nothing about **entitlement**, about whether the claimant is the driver for
whatever is behind them. The port registry cannot answer that, because a port
range is not a device: it has no identity, no vendor, and no way to be asked
what it is.

`014` lists it as the first of the phase's open decisions: *does a device claim
replace a port claim, or sit above it?*

**Decision.** It contains it. A driver claims a **function** — a bus, device
and function triple — and the claim carries every window that function decodes
with it. A bare port claim inside a claimed device's I/O window is then refused
by the port registry itself, because the device already holds those ports.

Not "authorises claiming", which is the alternative `014` names, because that
leaves two mechanisms and a gap between them: a driver could hold a device
capability and never claim the ports, or claim ports it has no device for, and
nothing would connect the two. One object holding both makes D-138's
permanence a property of the device rather than of every range separately.

**Claims are permanent**, exactly as port claims are. There is no release, so
there is no window in which a device is half-claimed, and a refusal strands
nothing: the holder is unaffected and the refused caller has taken nothing.

**Rationale, and five things the specification decides rather than taste.**

**CF8/CFC, not ECAM.** ECAM needs an MCFG table; QEMU's i440fx publishes none
and VMware emulates a 440BX. The 1992 mechanism is the only one guaranteed on
**both** targets D-18 makes this project answer to, and it reaches every byte a
read-only driver needs. An ECAM path later is purely additive (`012` §2).

**Enumeration must recurse, and the check machine had to be given something to
recurse into.** q35 as QEMU ships it has no PCI-to-PCI bridge, so a kernel that
scanned bus zero and stopped would have found all six functions, reported a
plausible number, and passed — while being unable to find a controller on a
real machine, because VMware places functions behind `pciBridge0`–`pciBridge7`.
The check machine now gets a `pci-bridge`, and the survey counts **buses**, not
only functions: the function count says nothing about descent and the bus count
says everything.

**A bridge has two base address registers, not six.** A type 1 header puts its
primary, secondary and subordinate bus numbers where a device's third register
would be. A claim that read six would decode those bus numbers as a window and
then *probe* it — writing all ones into the register that says which bus is
behind this bridge. The probe restores what it found, so nothing downstream
would necessarily notice; the check asserts the claim decodes two.

**The window size is complemented in the register's own width.** The first
version had one `window_size` taking "pass all ones for a narrow register",
and the I/O path then complemented a thirty-two-bit mask in sixty-four bits and
answered eighteen quintillion. Caught by a doctest on its first run. It is two
functions now — `memory_window_size` with an explicit `wide`, and
`io_window_size` returning a `u16` — because a width you must remember to pass
correctly is a width that will be passed wrongly.

**Bus master is set by the thing that holds the claim.** Without it a
controller cannot DMA, queues never advance, and *there is no error anywhere to
read* (`012` §2). It is the most expensive bit in this subsystem to forget, so
the claim owns it rather than a driver that might not.

**The lock check earned its keep on the first boot.** `pci::init` took this
module's guard and then called `port::claim_pair`, which takes the port
registry's — leaf under leaf, which is exactly what `lock_leaf` forbids and
what D-121's machine check watches for. It went red on the first run, naming
both sites. The claim happens outside the guard now.

**Consequences.**

- `libs/pci-config`: address encoding, header offsets, class codes, header
  types, BAR decoding, window sizing. Fifteen host tests and three doctests,
  five deliberate breaks, all red.
- `wost::arch::pci`: the two ports, claimed as a pair (D-138), and the probe
  that restores what it found. A register left holding `0xFFFF_FFFF` is a
  device claiming four gigabytes at the top of memory, and the next thing to
  enumerate believes it.
- `wost::arch::port::claim_span`: a run whose length the *machine* supplies.
  `claim_block` wants a constant, which is right for a PIT with three ports and
  a UART with eight; a device's I/O window is whatever the probe says.
- `kernel::pci`: an iterative walk over a bus worklist, each bus visited once —
  recursion would put a bridge topology on a thirty-two-kilobyte stack, and a
  bridge that names itself as its own secondary bus is a cycle.
- Three machine checks, five breaks, all red.
- **The `Device` is a kernel-side capability, not yet a handle.** It is
  obtainable only from the registry, exactly as `Port<T>` is, which is the same
  discipline one level up. It becomes something a program can hold in slice 8,
  where devfs is what hands it out — and that is the first moment there is
  anyone to hand it to (D-41).

## D-169 An NVMe controller, polled, and a disk with something on it — Status: Accepted (2026-08-25)

**Context.** `012` §2 disqualified virtio-blk rather than deprioritising it:
VMware exposes no virtio device of any kind, so under D-18 it could never close
a phase, and none of the virtqueue work carries to a driver that could. AHCI
spans both hosts and is a worse first driver for the same prerequisites. NVMe
is the only overlap between the two boot targets with a freely published
specification.

**Decision.** One controller, one I/O queue pair, polled, read-only.

**Polled is what makes it affordable.** `Create I/O Completion Queue` with
`IEN = 0` disables interrupts for that queue, which removes "this kernel has no
interrupt sharing" from the critical path entirely. The cost is that a read
spins, and slice 9 is where that becomes a submission and the spinning becomes
a wait.

**`CAP` is read, never assumed.** The queue size, the doorbell stride, the
timeout and the page-size range all come from it, and every one changes what a
correct driver does. `012` §11 records that VMware's values are unmeasured, so
a driver that assumed QEMU's would be a driver that fails the phase's
definition of done.

**The ring parallel is a trap, and the arithmetic is shared without the
structure** (`012` §3). Four things differ from `kernel::queue` and each is a
bug if assumed away: there is no completion tail to read, so entries are found
by their **phase tag**; completions are matched by **command identifier**, not
by slot; **capacity is `size - 1`**, whose symptom is a hang; and the doorbell
is **write-only**, so the tail is shadowed. `libs/nvme` holds the arithmetic;
there is no shared `Ring<T>`.

**Rationale, and the two places a check could not have failed.**

**A disk of zeros proves nothing.** The first version of the read check read
block zero out of a zero-filled backing file and compared it to zero — which is
what a buffer nobody wrote also contains. So `xtask` writes a **different
signature into every block**, and the check reads four of them: a driver that
ignored the logical block address would answer block zero to every request and
pass against a uniform disk. D-155's rule applied to a corpus rather than to an
argument.

**Reading one block too many is invisible at the front.** Every count in this
specification is zero-based, so `CDW12` carries `blocks - 1`. A driver that
sent the count unadjusted reads *two* blocks, and the first block's bytes are
still exactly right — the check went green against that mutation. It now clears
the buffer, reads one block, and requires the bytes **after** it to still be
zero. The extent of a read is only observable past its end.

**The queue wrap needs more commands than the queue has entries.** The phase
tag flips on every wrap, and a driver that never flipped it works perfectly
until the first one and then sees either every entry or none. The check does
three times the queue's depth, because the seam is the only place the defect
exists.

**Consequences.**

- `libs/nvme`: register offsets, `CAP` decoding, the `size - 1` capacity rule,
  doorbell placement, command building, completion decoding, and PRP
  construction. Thirteen host tests and three doctests, six deliberate breaks,
  all red.
- `kernel::nvme`: the initialisation sequence, `Identify Namespace`, one I/O
  queue pair with `IEN = 0`, and a read. Four machine checks, five breaks, all
  red.
- **The register window is mapped uncacheable**, which `map_device` already
  chose, and there is now a check that says so. `012` §2 names write-back
  mapping as a silent, hard-to-debug bug; an emulator forgives it and hardware
  does not, which is exactly the class of thing that has to be caught here.
- The check machine gains an NVMe controller and a two-megabyte backing file.
  The file is the *machine's*, not the disk image's: nothing on it is booted
  and the kernel writes nothing to it.
- The signature is written by `xtask` and looked for by `selftest.rs`, which is
  two transcriptions of one table — so a host test compares them (D-158).
- **A read still spins.** `POLL_ATTEMPTS` bounds it, because a controller that
  does not answer is a controller to report, and on a machine without bus
  master enabled a hang is the only symptom there is.

## D-170 FAT32, an oracle, and a block layer that is one trait — Status: Accepted (2026-08-25)

**Context.** `012` §1 chose the format on the **corpus**, not on the code, and
was blunt that the roadmap's premise was wrong: measured against GRUB's own two
readers — one project, one style, both read-only — FAT is 673 lines to ext2's
~772. Thirteen per cent. What decided it is that `xtask` already builds a FAT32
volume through `fatfs`, two volumes built seconds apart hash identically, every
pure-Rust ext2 image builder fails to run on this machine, and the reference
read-only ext2 crate generates its corpus by shelling out to `mkfs.ext2`, which
D-16 forbids.

`014` left two decisions open that this slice must take: **is the block layer a
trait or a concrete NVMe**, and **does a page cache exist in this phase**.

**Decision.**

**The block layer is a trait, and it belongs to the reader.** `libs/fat32`
defines `Sectors`, because that is the parameter the reader needs — not a
speculative kernel-side abstraction over a device this kernel has one of
(D-41). The host backs it with a byte slice and the kernel with the controller,
and the *same reader* runs over both. That is not a convenience: it is what
makes an oracle possible at all.

**The oracle is `fatfs` on the host.** Two independent implementations of one
format over one corpus, and the disagreement between them is the test. `012` §1
called this "a better arrangement than a dependency", and the reason is exact:
a dependency cannot disagree with itself.

**No page cache.** `014` carries it as a deliberate hole and the reasoning
holds: reading a directory means reading the same sectors repeatedly, whether
that is affordable is a *measurement*, and the measurement does not exist. A
cache is a decision for whoever has the numbers. The volume is also mounted per
call rather than kept, for the same reason — a boot sector is one sector and the
volume is not going anywhere.

**Rationale, and the corpus doing the work.**

`012` §1 warned that FAT's traps are differently shaped rather than fewer, and
three of them shaped the checks:

- **A cluster number is biased by two**, so the sector of a cluster is
  `first_data + (cluster - 2) * per_cluster`. Forget it and every file is read
  two clusters late, with no error anywhere.
- **A FAT32 entry is twenty-eight bits.** Compare all thirty-two against the
  end-of-chain marker and a chain that has ended is followed into whatever the
  reserved bits were — which is a cluster number, and reads as data.
- **A directory ends at a name beginning zero; `0xE5` is one deleted entry.**
  Confusing them either loses every file below or resurrects deleted ones.

**Four machine breaks passed at first, and the corpus is why.** The end-to-end
checks could not reach the twenty-eight-bit mask (the corpus's reserved bits are
zero), the file-size bound (the check's buffer was cut to the file's length),
the deleted marker (the corpus had no deletions) or the split first-cluster
field (every cluster fits sixteen bits). All four are caught by host tests over
synthetic entries, which is the right division — but two of them were reachable
on the machine for almost nothing, and a real disk is where a real disk lives:

- The corpus now creates a file and **deletes it**, leaving an `0xE5` slot
  *between* live entries. A reader that stopped there loses everything below,
  and the check counts the survivors.
- The read check now offers **a whole sector of buffer for a file that is not
  one**. A reader bounded by the cluster hands back its previous occupant's
  data, and a buffer cut to the file's length could never see it.

That is D-165 again — a check must reach the code it is about — arriving
through the *corpus* rather than through the set-up.

**Consequences.**

- `libs/fat32`: BPB parsing, the table, directory decoding, chain walking,
  per-call offsets, path resolution. No allocation and no panic on hostile
  input: every length checked against what remains, every cluster against the
  data area's bounds, and every walk bounded — a FAT is a linked list a hostile
  volume can make circular, and following one is a hang.
- Eight host tests in the crate, eight more in `xtask::image::corpus` including
  the oracle, four machine checks. Six breaks, all red; two of them made
  reachable on the machine by improving the corpus.
- **512-byte sectors only**, and a volume that says otherwise is refused rather
  than misread. That is what lets every buffer be a fixed array: supporting 4096
  would need a caller-supplied buffer everywhere or an allocator, and no volume
  this project builds or boots uses one.
- The volume sits at **sector 2048** of the namespace, where a partition table
  would put it, and the reader takes a base rather than assuming zero — a reader
  that could not be given an offset is one nothing can be put in front of.
- Three transcriptions meet: the corpus's names and contents, the kernel's copy
  of them, and the base sector stated in two files. A host test compares all of
  them (D-158).
- **The corpus still has one writer.** `014` budgets for it: until a
  foreign-produced FAT32 image is obtained, a shared misreading of the
  specification would pass both implementations. That is the honest limit of an
  oracle built from one tool.
