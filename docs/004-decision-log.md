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

## D-31 A fault is terminal and reported; a breakpoint resumes — Status: Accepted (2026-08-13) (amended by D-77)

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

## D-65 The gate boots three kernels — Status: Accepted (2026-08-23)

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
the arithmetic in `libs/xstate`.

- **Eager only. Never lazy.**
- **`XSAVES` → `XSAVEC` → `XSAVE`, and never `XSAVEOPT`.**
- **`XCR0` = x87 + SSE + AVX.** No AVX-512, no AMX.
- **`XstateBuf` out-of-line and growable**, 64-byte aligned, sized at boot from
  the mask — never a fixed array inside the thread struct.
- **Offsets are computed from `CPUID.0DH` into a table** in `libs/xstate`,
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

- `libs/xstate` is a new host-tested crate; the APX snapshot is one of its
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
