# The Wizard's OS 🧙

A from-scratch operating system: Rust kernel, x86_64, UEFI, booted by
[Limine](https://github.com/limine-bootloader/limine). **Capability-based
and async-first** — handles + submission/completion queues are the native
ABI (POSIX is just a translation layer for ports), `spawn()` not `fork()`,
CoW checksummed filesystem (WizFS), WebAssembly + `wasip1` as first-class
apps (D-72).
Multi-year project; destination: a full userspace with our own compositor,
a ported Python, and NetSurf rendering the document web (D-75).
See [docs/002-architecture.md](docs/002-architecture.md).

Design docs live in [docs/](docs/) — one per subsystem, written before the
code (design-first). Start with the
[decision log](docs/004-decision-log.md) (every choice, its alternatives,
and rationale) and the [architecture](docs/002-architecture.md). Research
evidence behind them is in [the founding pass](docs/003-research-notes.md),
[the second pass](docs/008-research-notes-2.md), and
[the foundation-verification pass](docs/010-research-notes-3.md); later passes
annotate corrections to the earlier historical snapshots.
[Code conventions](docs/005-code-conventions.md) covers how the code is written.

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
cargo xtask ci       # the gate: fmt --check, clippy, doc, tests, vendor digests,
                     #   then four boots — test-boot (release), test-boot
                     #   (debug, overflow checks on), boot-check (D-65), and
                     #   degraded-check (no x2apic, D-83)
```

`ci` is the only gate — there is no hosted CI (D-26). `cargo xtask help` lists
every task and is the authority; a check requires this paragraph to name the
same set, because a task the README does not mention is a task nobody runs.
`boot-check` boots the shipped kernel and requires it to settle at its prompt;
`degraded-check` boots it on a processor with no x2APIC and requires it to
report that and settle rather than triple-fault.

Three tasks are deliberately **not** in `ci`, each because it needs a tool this
project cannot vendor into a Rust build: `model-check` (TLC, needs Java),
`vmware` (needs VMware Workstation to boot the result), and `mutate` (one QEMU
boot per known defect, which is minutes rather than seconds). A gate step that
fails for want of a tool teaches people to skip the gate.

`test` runs the host suite; `test-boot` boots a kernel built with the
`selftest` feature headless in QEMU, injects a keystroke over QMP when the
kernel asks for one (D-38), and reads the verdict from the exit code, which
is how anything needing a real machine gets tested (D-28). `doc` exists to
make the rustdoc link lints fire — nothing else in the gate runs rustdoc, so
without it a doc link can rot unnoticed (D-22).

Every cargo-driven check runs over **three** configurations, not two: the
host packages, the kernel packages, and the kernel packages again with
`selftest` on. A cargo feature is a different crate graph, and linting only
one of them is how two dead constants once rode a release image out. Adding a
feature means adding a pass to `PASSES` in `xtask/src/tasks.rs`.

Every kernel build is read back and checked against what `kernel/linker.ld` is
written to produce — entry in the higher half, no orphan sections, no segment
both writable and executable, `.bss` costing no file space (D-40).

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
libs/       shared no_std crates: pure logic, host-testable (acpi-tables, eevdf,
            frame-space, pixel, port-space, ps2-keys, slab-layout, stack-layout,
            timebase, x86-apic, x86-paging, x86-pit, x86-tables)
xtask/      build orchestrator: pure-Rust disk imaging, QEMU/VMware runners
boot/       limine.conf + vendored Limine UEFI binary (digest-pinned)
docs/       design docs (design-first: docs before code)
userspace/  (future) native userland: init, shell, coreutils
ports/      (future) recipes for mlibc, CPython, links, NetSurf
```
