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
deliberately **not** enabled. `cargo xtask ci` is the single gate, and
`.github/workflows/ci.yml` runs the same steps on Linux.

**Rationale.** `undocumented_unsafe_blocks` is what mechanically forces a
`// SAFETY:` on every unsafe operation, and its companion keeps each comment
covering exactly one operation. Together they turn D-13's discipline from an
aspiration into a build failure. The excluded lint is the one that would
demand a doc comment on every private helper, which is noise; the policy is
"prose where it carries a safety obligation, nowhere else"
(`005-code-conventions.md`).

**Consequences.** Cargo forbids mixing `[lints] workspace = true` with local
lint tables in the same manifest, so per-crate escalation goes in source
attributes — `#![deny(unsafe_code)]` sits at the top of `kernel/src/main.rs`,
where a reader meets it in the first five lines. `xtask` opts out of the
workspace lints entirely. Automated tests exist for host-side logic only; the
reasoning and the path forward are in `005-code-conventions.md`.

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
the image step runs on Linux CI. New tasks: `check`, `clippy`, `fmt`, `test`,
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

**Consequences.** Updating Limine is now a two-line diff a reviewer can
actually check. Every future vendored artifact gets the same treatment.

---

## Cross-references

- `000-vision.md` — destination + phase roadmap
- `001-boot.md` — boot contract and disk layout (D-03, D-16)
- `002-architecture.md` — the native architecture (D-06..D-15)
- `003-research-notes.md` — evidence tiers and citations behind
  Verified statuses
- `005-code-conventions.md` — how the code is written (D-21, D-22, D-23)
