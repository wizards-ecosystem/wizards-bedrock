<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/bedrock-logo-dark.png">
    <img src="docs/assets/bedrock-logo.png" alt="The Wizard's Bedrock" width="420">
  </picture>
</h1>

Bedrock is a from-scratch **Rust kernel** for x86_64 and UEFI. [Limine](https://github.com/limine-bootloader/limine) enters it in long mode at `0xffffffff80000000`. From that handoff the kernel installs its own descriptor tables and page tables, turns on the CPU's supervisor protections, schedules threads, fences PCI DMA with VT-d, reads an NVMe disk, and runs a ring-3 program whose external authority is a handle table.

The workspace version is 0.2.0. The qualified boot image for that version is recorded in [docs/releases/v0.2.0.md](docs/releases/v0.2.0.md). This repository is the kernel: hardware, memory, scheduling, storage, and the capability ABI. A shell, desktop, and network stack live outside it. The phase plan is [docs/000-vision.md](docs/000-vision.md).

## What a boot does

`cargo xtask run` builds a 64 MiB UEFI disk and boots the shipped kernel in QEMU. The image is a raw MBR disk with one FAT32 EFI system partition, written entirely in Rust. Limine loads `wizard-kernel.elf` and the module `hello.elf`.

On the way to the prompt the kernel:

1. Fences inherited PCI bus masters before it trusts a device, and installs a default-deny VT-d domain before storage DMA.
2. Opens a 16550 serial log at `COM1` (`0x3F8`) and, when Limine provides a linear framebuffer, a Noto Sans Mono console with its own shadow buffer.
3. Installs the GDT, IDT, and TSS, then enables write protection, NX, SMEP, SMAP, and UMIP where the processor reports them. `syscall` and `sysenter` are shut. The only gate ring 3 may enter is interrupt `0x80`.
4. Enables XSAVE, compacted when the CPU allows it. Without extended state, ring 3 stays closed.
5. Builds a complete physical memory map, a two-bitmap frame allocator, W^X page tables, a direct map, and a heap whose size classes run from 16 to 2048 bytes. Page-table pages, heap frames, and user stacks are allocated zeroed. The boot thread then moves onto a guarded stack. If that allocation fails, ring 3 stays closed and the kernel continues.
6. Silences the legacy PICs, reads speculation controls, and selects user page-table isolation (below). It starts the local APIC in x2APIC mode, calibrates the APIC timer and TSC from the PIT, and publishes a read-only time page.
7. Walks PCI configuration space through bridges, claims one NVMe function, and mounts its FAT32 volume read-only. Without a complete VT-d domain the log says `storage: no DMA isolation` and the volume stays unmounted.
8. Brings up tmpfs and a devfs that names `storage` (a block device) and `console` (a character device).
9. Starts a tickless EEVDF scheduler and an idle thread that halts. It loads `hello.elf` into its own address space: one read-execute segment linked at `0x400000000000`, a guarded stack, the time page, a shared submission/completion ring, and a handle table whose single birth handle is an opaque bootstrap identity. The witness checks entry version 1, posts `Echo` with argument `0x00C0FFEE`, enters the kernel, checks the completion, and exits.
10. The boot thread blocks on the PS/2 keyboard and paints each decoded character. With no PCI topology the I/O APIC routes that line and each key is an interrupt. While PCI is present, interrupt isolation keeps those lines masked and a poller drains the controller every 10 ms.

Serial carries diagnostics. The framebuffer carries the banner and the echo.

## How the kernel is put together

### One crate may write `unsafe`

`wost` is the privileged framework: Limine glue, ports, MMIO, paging, the VT-d unit, interrupt entry, context switch, and locks. It is the only crate allowed to write `unsafe`. The kernel binary is `no_std`, `no_main`, and `#![deny(unsafe_code)]`, because `wost::entry!` expands its boot glue there. Every `libs/` crate is `#![forbid(unsafe_code)]`. Arithmetic that can be checked without hardware lives in `libs/` and runs under `cargo test` on the host. `wost` keeps the lock and the instruction.

Every kernel link is read back. The entry sits in the higher half, no section is orphaned, no segment is both writable and executable, and `.bss` costs no file bytes.

### Processor, memory, and time

The kernel runs on the boot processor. x2APIC is required; a CPU without it leaves interrupts off. The scheduler's timer is an alarm, armed for the next decision and stopped when nothing is due. A 3 ms slice is the EEVDF tunable. Sixteen kernel stacks of 32 KiB sit above an unmapped 4 KiB guard, plus the boot thread. The double-fault stack and the boot thread's own stack use the same geometry.

User address spaces share the kernel half when the CPU's speculation controls admit it. When they do not, each user space gets a paired root: a full kernel root and a stripped root that maps only the linker-owned entry enclave. The self-test kernel forces the paired path so that entry is executed even on a CPU that would allow sharing. If neither path is available, ring 3 stays closed. Returns to ring 3 execute `verw` unless the processor reports `MDS_NO`.

The time page, mapped at `0x10000000000` in every user space, publishes a fixed tick-to-nanosecond conversion under a seqlock. Constants on that page (version, page size, counter rate) are written once. Two consecutive disagreements with the PIT freeze public time.

### The native ABI

A program's external authority is its handle table. Handles are process-local integers: an 18-bit arena index (262,144 slots), a generation that rejects stale reuse, and a per-table mixer. Rights name what the holder may do. `Duplicate` returns a new handle with fewer rights. A spawned program is born holding exactly one handle, an opaque channel identity with no peer and no messages.

The data path is a submission/completion ring, protocol version 1: 8 submission slots and 16 completion slots, kernel-owned pages mapped into the process. The kernel copies shared bytes once and validates the copy. Every accepted submission reserves a completion, and every operation posts exactly one. The opcodes are:

| Opcode | What it does |
|---|---|
| `Echo` | Return the argument |
| `Uptime` | Return kernel uptime in nanoseconds |
| `Duplicate` | Narrow a held handle; the completion carries the new one |
| `Timer` | Complete after the requested nanoseconds |
| `Cancel` | End an in-flight operation by token; both the cancel and the target still complete once |
| `OpenAt` | Resolve a path from a directory handle and return a handle for it |
| `Read` | Read a file at an explicit offset into the ring's data page |

Anything else completes as a refusal. Paths are relative to a held directory. `..` is resolved lexically and underflow is refused, so the resolver has no parent pointer to follow. FAT32 names are 8.3; long-name entries and the volume label are skipped. File reads have no cursor: each `Read` says where it starts.

Four synchronous calls cross interrupt `0x80`: `Exit`, `RingEnter` (serve the ring and report how many operations completed), `RingWait` (sleep until a deferred completion is due), and `ThreadBase` (set this thread's `FS` base). `RingEnter` admits an NVMe read; one file worker polls the controller and posts the completion.

The same ring protocol is a TLA+ model in `models/`. `Ring.tla` checks admission. `RingMemory.tla` checks publication across normal, cancelled, and retired operations. Four mutant configs must each violate a named invariant. `cargo xtask model-check` runs TLC against the digest-pinned jar in `tools/tla/`.

`hello.elf` is the fixed ELF64 witness the image builder emits. It is the program the shipped boot runs. It is a boundary exercise, small enough that a wrong answer hits `ud2`.

### Storage

PCI enumeration uses CF8/CFC and walks bridges, because a scan of bus zero misses controllers behind them. A driver claims a function, not a bare port range. The first usable NVMe controller is brought up with one I/O queue pair and completion interrupts off. Queue depth, doorbell stride, timeout, and page size come from `CAP`. The register window is mapped uncacheable, and bus master stays off until the controller-disable readback matches. `libs/nvme` holds the command and queue arithmetic; the kernel holds the frames and the poll.

The FAT32 reader is the same `no_std` crate on the host and in the kernel. It accepts 512-byte sectors, checks cluster chains, and allocates nothing. The volume begins at sector 2048, the offset the image builder writes. The VFS is a directory enum over FAT32, tmpfs, and devfs: resolve, enumerate, and a reserved modify. Backends are selected by an enum, so a new one is a compile error at every match.

## Layout

```
kernel/          kernel binary, higher-half, no hand-written unsafe
wost/            privileged framework, the only crate that may write unsafe
libs/            host-tested no_std arithmetic
xtask/           build, image, QEMU, VMware, gate, mutation, evidence
boot/            limine.conf and a digest-pinned Limine UEFI binary
models/          TLA+ ring models and the four mutant configs
tools/           pinned TLC jar and a digest-checked FAT32 corpus
docs/            design docs, one per subsystem
supply-chain/    SPDX SBOM, third-party notices, license texts
fixtures/abi-pressure/   native ABI transcription
handoff/         route-neutral envelope fixtures
experiments/     optional host tools outside the workspace and the gate
```

| Crate | What it owns |
|---|---|
| `acpi-tables` | RSDP, XSDT, MADT, DMAR, validated before use |
| `eevdf` | virtual-deadline scheduling |
| `elf64` | ELF64 headers and loadable segments |
| `fat32` | read-only FAT32 over any sector source |
| `frame-space` | physical-frame bitmap allocation |
| `handle-space` | handle encoding, rights, quotas |
| `memory-map` | a firmware map that refuses rather than truncate |
| `mmio-range` | repeated MMIO span checks |
| `nvme` | NVMe registers, queues, and commands |
| `panic-record` | framed panic text |
| `path-walk` | lexical paths that cannot ascend |
| `pci-config` | configuration-space addresses and BARs |
| `pixel` | framebuffer pixel layout |
| `port-space` | I/O port reservations, including wide `out`s |
| `ps2-keys` | PS/2 set 2 scancodes and the US layout |
| `register-bits` | register widths, with reserved high bits ignored on read |
| `ring-proto` | ring admission and shared-memory layout |
| `slab-layout` | heap size classes |
| `stack-layout` | guarded stack geometry |
| `timebase` | tick and nanosecond conversion |
| `trap-cause` | exception vector and error-code decode |
| `vtd` | VT-d tables, registers, and invalidation |
| `wizzy-handoff` | route-neutral artifact identity envelopes |
| `x86-apic` | local APIC registers |
| `x86-cpuid` | CPUID leaves, including protection and x2APIC |
| `x86-paging` | four-level page-table entries |
| `x86-pit` | PIT commands and calibration arithmetic |
| `x86-tables` | GDT, IDT, and TSS encodings |

## Quick start

The documented host is Windows with the GNU Rust toolchain. The pinned channel is `1.97.1` (`rust-toolchain.toml`), which also installs `rustfmt`, `clippy`, and the `x86_64-unknown-none` target. QEMU is optional for the compile.

```powershell
git clone https://github.com/wizards-ecosystem/wizards-bedrock.git
cd wizards-bedrock
cargo fetch --locked
cargo xtask build --release
```

A successful build compiles the kernel and checks the linked image. Boot it with `cargo xtask run`. The full local gate is `cargo xtask ci`.

## Building & running

Install rustup with the GNU host (`rustup-init -y --default-host x86_64-pc-windows-gnu`). Open a fresh terminal so `cargo` is on `PATH`. The first build installs the pinned toolchain from `rust-toolchain.toml`. That file omits `rust-mingw` because the component does not exist on other hosts; rustup installs it with the Windows GNU toolchain. For boots, install QEMU (`winget install SoftwareFreedomConservancy.QEMU`). The runner looks at `QEMU`, then `QEMU_DIR`, then `C:\Program Files\qemu`, then `PATH`.

```
cargo xtask build --release   # compile the kernel
cargo xtask image              # write target/bedrock.img
cargo xtask run                # boot the shipped image in QEMU, serial on stdio
cargo xtask vmware             # generate vm/self-test/
cargo xtask vmware --shipped   # generate vm/shipped/
cargo xtask help               # every task
```

`cargo xtask run` and `cargo xtask image` use the shipped kernel. `cargo xtask run --` forwards QEMU diagnostic options `-d`, `-D`, `-display`, and `--display`, each with a value. The machine profile stays fixed.

`cargo xtask debug` opens a private QEMU at `kmain` under the pinned WinLibs GDB. `cargo xtask debug --check` proves the setup and exits. The accepted identity is GDB 17.2 from WinLibs r4:

```
winget install --id BrechtSanders.WinLibs.POSIX.UCRT --version 16.1.0-14.0.0-r4 --exact
```

Set `GDB` if that `gdb.exe` is not on `PATH`.

**Never set `RUSTFLAGS`.** It replaces the `[target.x86_64-unknown-none]` rustflags in `.cargo/config.toml`, and the kernel links without `code-model=kernel`. Pass lint flags after `--`.

`cargo build --workspace` cannot select a target per package on stable. Bare Cargo commands cover the host half through `default-members`. These aliases reach the kernel:

| Alias | Runs |
|---|---|
| `cargo kcheck` | `cargo check` on `wost` and `wizard-kernel` for `x86_64-unknown-none` |
| `cargo kclippy` | the same for clippy |
| `cargo kbuild` | a release kernel build |

`cargo xtask` runs the orchestrator under the optimized `xtask` profile. Audited graphs, host tests, and the mutation workspace keep the plain `dev` profile. `.vscode/settings.json` points rust-analyzer at `cargo xtask check`.

Release builds use `panic = "abort"`, LTO, and debug info that costs no loaded bytes.

## Development

```
cargo xtask ci
```

`ci` is the local source gate. Its stages are rendered from the same registry the tool executes:

<!-- CI-REGISTRY:START -->
Cargo graphs used by each Cargo-driven gate stage:

| Graph | Target | Features |
|---|---|---|
| host | `host` | `none` |
| kernel shipped | `x86_64-unknown-none` | `none` |
| kernel self-test | `x86_64-unknown-none` | `wizard-kernel/selftest` |
| kernel refuse-pci | `x86_64-unknown-none` | `wizard-kernel/refuse-pci` |

Ordered `cargo xtask ci` stages:

| Order | Invocation | Purpose |
|---:|---|---|
| 1 | `cargo xtask fmt --check` | refuse unformatted source |
| 2 | `cargo xtask clippy` | lint every registered Cargo graph |
| 3 | `cargo xtask api-audit` | prove every public foundation API has a caller |
| 4 | `cargo xtask doc` | build docs for every registered Cargo graph |
| 5 | `cargo xtask test` | run host tests and doctests |
| 6 | `cargo xtask supply-chain` | bind locked bytes, licenses, scope and SPDX |
| 7 | `cargo xtask verify-vendor` | verify every vendored digest |
| 8 | `cargo xtask test-boot --release` | run all machine checks on the release kernel |
| 9 | `cargo xtask test-boot --debug` | run all machine checks with overflow checks enabled |
| 10 | `cargo xtask boot-check --release` | boot, exercise and settle the shipped image |
| 11 | `cargo xtask degraded-check --release` | prove the no-PCI, no-x2APIC and no-IOMMU refusals |
<!-- CI-REGISTRY:END -->

`boot-check` boots the shipped image and requires the keyboard prompt. `degraded-check` requires the kernel to refuse a machine with no PCI configuration and a machine with no x2APIC, to bring up tmpfs and devfs with no hardware behind them, and to leave storage unmounted when no remapping unit exists. `test-boot` boots the `selftest` kernel headless and reads the verdict from QEMU's exit code. `api-audit` requires a compiler-resolved caller for every public item in `libs/` and `wost`. `doc` exists so rustdoc's link lints run.

These tasks stay outside `ci` because they need a tool, a person, or a long run: `debug`, `baseline` (six QEMU boots), `model-check`, `vmware`, `mutate` (one QEMU boot per registered defect, on a copy under `target/mutate/`), `qualify-advisory`, `qualify-mutants-grade --run`, and `qualify-security-deep --run`. After a human boots a generated VMware machine, `cargo xtask qualify-vmware` checks the self-test transcript and `cargo xtask qualify-vmware --shipped` checks the shipped one. `cargo xtask evidence` runs the gate, reproduces the shipped image twice, and writes a bundle that `cargo xtask verify-evidence` rechecks offline.

## Further reading

See [docs/002-architecture.md](docs/002-architecture.md). Subsystem docs, written before the code:

- [Boot](docs/001-boot.md)
- [CPU](docs/006-cpu-fundamentals.md)
- [Memory](docs/007-memory.md)
- [Scheduling](docs/009-multitasking.md)
- [Ring 3 and the ABI](docs/011-userspace.md)
- [Storage](docs/014-storage.md)
- [Code conventions](docs/005-code-conventions.md)

The language boundary is [docs/015-wizzy-userland.md](docs/015-wizzy-userland.md) (D-183). The kernel register is [docs/016-pre-wizzy-foundation.md](docs/016-pre-wizzy-foundation.md) (D-184).

Outside contributions arrive through forks and pull requests. [CONTRIBUTING.md](CONTRIBUTING.md) covers scope and which checks a change needs. Report vulnerabilities through [SECURITY.md](SECURITY.md).
