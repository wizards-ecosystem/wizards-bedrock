# The Wizard's OS 🧙

A from-scratch operating system: Rust kernel, x86_64, UEFI, booted by
[Limine](https://github.com/limine-bootloader/limine). **Capability-based
and async-first** — handles + submission/completion queues are the native
ABI (POSIX is just a translation layer for ports), `spawn()` not `fork()`,
CoW checksummed filesystem (WizFS), WASI components as first-class apps.
Multi-year project; destination: a full userspace with our own compositor,
a ported Python, and a web browser. See [docs/002-architecture.md](docs/002-architecture.md).

Design docs live in [docs/](docs/) — one per subsystem, written before the
code (design-first). Start with the
[decision log](docs/004-decision-log.md) (every choice, its alternatives,
and rationale) and the [architecture](docs/002-architecture.md); research
evidence behind them is in [research notes](docs/003-research-notes.md).

## Building & running

Everything is self-contained on Windows — no WSL, no external imaging tools.
Prerequisites: [rustup](https://rustup.rs) (GNU host toolchain) and
[QEMU](https://www.qemu.org/) (`winget install SoftwareFreedomConservancy.QEMU`).

```
cargo xtask build    # compile the kernel (x86_64-unknown-none)
cargo xtask image    # build target/wizard-os.img — bootable UEFI disk image
cargo xtask run      # boot it in QEMU (serial output on stdio)
cargo xtask vmware   # generate vm/wizard-os.vmx + VMDK for VMware Workstation
```

## Layout

```
kernel/     the kernel (no_std Rust, higher-half, Limine protocol)
xtask/      build orchestrator: pure-Rust disk imaging, QEMU/VMware runners
boot/       limine.conf + vendored Limine UEFI binary
docs/       design docs (design-first: docs before code)
libs/       (future) shared no_std crates, host-testable (e.g. WizFS format)
userspace/  (future) native userland: init, shell, coreutils
ports/      (future) recipes for mlibc, CPython, links, NetSurf
```
