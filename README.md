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
evidence behind them is in [research notes](docs/003-research-notes.md), and
[code conventions](docs/005-code-conventions.md) covers how the code is written.

## Building & running

Everything is self-contained on Windows — no WSL, no external imaging tools.
Prerequisites: [rustup](https://rustup.rs) (GNU host toolchain:
`rustup-init -y --default-host x86_64-pc-windows-gnu`, no Visual Studio
needed) and [QEMU](https://www.qemu.org/)
(`winget install SoftwareFreedomConservancy.QEMU`). Open a fresh terminal
after installing rustup so `cargo` is on PATH; the pinned toolchain and
`x86_64-unknown-none` target install automatically on first build via
`rust-toolchain.toml`.

```
cargo xtask build    # compile the kernel (x86_64-unknown-none)
cargo xtask image    # build target/wizard-os.img — bootable UEFI disk image
cargo xtask run      # boot it in QEMU (serial output on stdio)
cargo xtask vmware   # generate vm/wizard-os.vmx + VMDK for VMware Workstation
cargo xtask help     # every task, including the check tasks below
```

`cargo xtask run -- <args>` passes arguments straight through to QEMU.
QEMU is found at `C:\Program Files\qemu` or on `PATH`; `QEMU_DIR` or `QEMU`
override that.

## Development

```
cargo xtask ci       # the gate: fmt --check, clippy, tests, vendor digests, image
```

`ci` is the only gate — there is no hosted CI (D-26). Individual pieces:
`cargo xtask fmt`, `cargo xtask clippy`, `cargo xtask test`,
`cargo xtask test-boot`, `cargo xtask verify-vendor`, `cargo xtask clean [--all]`.

`test` runs the host suite; `test-boot` boots a kernel built with the
`selftest` feature headless in QEMU and reads its verdict from the exit code,
which is how anything needing a real machine gets tested (D-28).

**`cargo build --workspace` does not work, and cannot on stable.** The kernel
crates only build for `x86_64-unknown-none` and `xtask` only for the host, and
cargo has no stable way to say so per package (`package.forced-target` is
nightly). Bare cargo commands are therefore scoped to the host half by
`default-members`, and these aliases reach the other half:

| Alias | Runs |
|---|---|
| `cargo kcheck` | `cargo check` on `wost` + `wizard-kernel` for the kernel target |
| `cargo kclippy` | the same for clippy |
| `cargo kbuild` | a release kernel build |

**Never set the `RUSTFLAGS` environment variable here.** It replaces the
`[target.x86_64-unknown-none]` rustflags in `.cargo/config.toml`, and the
kernel would link without `code-model=kernel` — a broken higher-half image
with no build error. Pass lint flags after `--` instead.

For the same reason, `.vscode/settings.json` points rust-analyzer at
`cargo xtask check`. Without it the editor checks the kernel for the host
triple and reports phantom errors across every kernel file.

## Layout

```
wost/       privileged kernel framework — the only crate that may write `unsafe`
kernel/     the kernel binary (no_std, #![deny(unsafe_code)], higher-half)
libs/       shared no_std crates: pure logic, host-testable (pixel, x86-apic,
            x86-tables)
xtask/      build orchestrator: pure-Rust disk imaging, QEMU/VMware runners
boot/       limine.conf + vendored Limine UEFI binary (digest-pinned)
docs/       design docs (design-first: docs before code)
userspace/  (future) native userland: init, shell, coreutils
ports/      (future) recipes for mlibc, CPython, links, NetSurf
```
