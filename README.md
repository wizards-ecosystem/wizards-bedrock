# The Wizard's OS 🧙

A from-scratch operating system: Rust kernel, x86_64, UEFI, booted by
[Limine](https://github.com/limine-bootloader/limine). **Capability-based
and async-first** — handles + submission/completion queues are the native
ABI (POSIX is just a translation layer for ports), `spawn()` not `fork()`,
CoW checksummed filesystem (WizFS), WebAssembly + `wasip1` as first-class
apps (D-72). D-183 fixes the language boundary: the kernel remains Rust, and
every feasible first-party target-side non-kernel component is written in
Wizzy once its compiler-readiness gate opens. Phase 7 product userland stops at
that gate; boundary probes, ABI/platform preparation, and a narrow audited
runtime shim may make it ready. Host tooling, third-party libraries and ports,
and compatibility runtimes keep their appropriate languages.
The finite work still permitted before that gate is the D-184
[pre-Wizzy foundation register](docs/016-pre-wizzy-foundation.md); it separates
confirmed kernel defects and missing qualification evidence from trigger-only
and post-gate product work.
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
cargo xtask ci       # run the complete local gate below
```

`ci` is the only gate — there is no hosted CI (D-26). `cargo xtask help` lists
every task and renders this same registry. The marked section is checked byte
for byte against the registry that execution iterates, so changing a Cargo
graph or gate stage requires a reviewable documentation change (D-212).

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
| 6 | `cargo xtask verify-vendor` | verify every vendored digest |
| 7 | `cargo xtask test-boot --release` | run all machine checks on the release kernel |
| 8 | `cargo xtask test-boot --debug` | run all machine checks with overflow checks enabled |
| 9 | `cargo xtask boot-check --release` | boot, exercise and settle the shipped image |
| 10 | `cargo xtask degraded-check --release` | prove the no-PCI and no-x2APIC refusals |
<!-- CI-REGISTRY:END -->

`boot-check` boots the shipped kernel and requires it to settle at its prompt;
`degraded-check` boots it with PCI configuration forced unavailable and on a
processor with no x2APIC. It requires both refusals, the hardware-free tmpfs and
devfs surveys, and a steady state rather than a triple fault.

Three tasks are deliberately **not** in `ci`, each because it needs a tool this
project cannot vendor into a Rust build: `model-check` (TLC, needs Java),
`vmware` (needs VMware Workstation to boot the result), and `mutate` (one QEMU
boot per known defect, which is minutes rather than seconds). A gate step that
fails for want of a tool teaches people to skip the gate. `model-check` prints
the exact resolved Java identity before invoking it; `vmware` prints the
resolved `vmrun` identity and a fully quoted launch command when Workstation is
installed, or names that dependency as unavailable without blocking VM
generation (D-218).

`mutate` requires a clean `HEAD` and writes only to a locked, journalled copy
under `target/mutate/`, with its own build directory. An interrupted sweep
resumes at its active mutation after restoring that copy; it never repairs or
rewrites the checkout or Git index (D-208).

`cargo xtask evidence` requires a clean `HEAD`, runs the complete gate, builds
the shipped release in two fresh target directories, requires byte-identical
kernel and image artifacts, and boots the exact bundled image. It writes the
deterministic bundle under `target/evidence/`; `cargo xtask verify-evidence`
then rechecks its source identity, installed tool and firmware identities,
vendored and machine inputs, artifact hashes, reproducibility result and boot
result without network access. The QEMU hardware ABI is pinned to
`pc-q35-9.2`, not the moving `q35` alias. Evidence schema v2 always carries
Java and VMware state, version and executable digest fields—even when either
tool is unavailable—so verification reports the exact missing or changed
qualification dependency rather than changing schema (D-218).

`test` runs the host suite; `test-boot` boots a kernel built with the
`selftest` feature headless in QEMU, injects a keystroke over QMP when the
kernel asks for one (D-38), and reads the verdict from the exit code, which
is how anything needing a real machine gets tested (D-28). `doc` exists to
make the rustdoc link lints fire — nothing else in the gate runs rustdoc, so
without it a doc link can rot unnoticed (D-22).

`api-audit` uses stable rustc's resolved deprecation diagnostics across every
registered Cargo graph to require a real caller for each public `libs/` and
`wost` item declaration or enum variant. It distinguishes same-name items and
does not treat re-exports or documentation as callers; fields remain outside
the claim because rustc does not diagnose every live destructuring pattern
(D-213).

Every Cargo-driven check runs over every graph in the registry above. A Cargo
feature is a different crate graph, and linting only one of them is how two
dead constants once rode a release image out. Adding a build-only feature
means adding its graph to `xtask/src/gate.rs`.

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
userland/   native ABI header + the uncompiled mlibc pressure test from Phase 5
ports/      (future) recipes for mlibc, CPython, links, NetSurf
```
