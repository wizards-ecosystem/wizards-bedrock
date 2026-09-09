<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/os-logo-dark.svg">
    <img src="docs/assets/os-logo.svg" alt="The Wizard's OS" width="420">
  </picture>
</h1>

A from-scratch operating system: Rust kernel, x86_64, UEFI, booted by
[Limine](https://github.com/limine-bootloader/limine). **Capability-based
and async-first**: handles + submission/completion queues are the native
ABI (POSIX is just a translation layer for ports), `spawn()` not `fork()`.
A CoW checksummed filesystem (WizFS) and WebAssembly with `wasip1` as a
first-class app format are designed and not built (D-72, D-73); storage today
is a read-only third-party FAT32.
D-183 fixes the language boundary: the kernel remains Rust, and
every feasible first-party target-side non-kernel component is written in
Wizzy once its compiler-readiness gate opens. Phase 7 product userland stops at
that gate; the OS may specify and validate the route-neutral handoff, while the
separable runtime remains owned and gated by `wizards-ink` and the OS adapter
remains post-gate integration. Host tooling, third-party libraries and ports,
and compatibility runtimes keep their appropriate languages.
The finite work still permitted before that gate is the D-184
[pre-Wizzy foundation register](docs/016-pre-wizzy-foundation.md); it separates
confirmed kernel defects and missing qualification evidence from trigger-only
and post-gate product work.
Multi-year project; destination: a full userspace with our own compositor,
a ported Python, and NetSurf rendering the document web (D-75).
See [docs/002-architecture.md](docs/002-architecture.md).

## Status

Working, and unfinished. The kernel boots on UEFI, brings up descriptor tables,
x2APIC, paging and an EEVDF scheduler, reaches ring 3 with a handle table and an
async queue ABI, claims a PCI NVMe device, and mounts read-only FAT32 over a
VFS. It then settles at a prompt that echoes keystrokes. There is no `init`, no
shell and no user programs: that is Phase 7, and it is gated.

| Phases 0 to 5 | Closed, each by a dated VMware boot. Last close 2026-08-24 |
| --- | --- |
| Phase 6, VFS and storage | Slices landed 2026-08-25. **Not closed**: qualification evidence is still open |
| Phase 7, userland | **Gated** on the Wizzy compiler-readiness gate (D-183), which is CLOSED |

[docs/000-vision.md](docs/000-vision.md) carries the phase-by-phase ledger, and
the D-184 [pre-Wizzy foundation register](docs/016-pre-wizzy-foundation.md) is
the only current backlog. `wizards-ink`, which owns the Wizzy compiler, is a
separate repository and is not yet public.

Design docs live in [docs/](docs/), one per subsystem, written before the
code (design-first). Start with the
[decision log](docs/004-decision-log.md) (every choice, its alternatives,
and rationale) and the [architecture](docs/002-architecture.md). Research
evidence behind them is in [the founding pass](docs/003-research-notes.md),
[the second pass](docs/008-research-notes-2.md), and
[the foundation-verification pass](docs/010-research-notes-3.md); later passes
annotate corrections to the earlier historical snapshots.
[Code conventions](docs/005-code-conventions.md) covers how the code is written.

## Building & running

Everything is self-contained on Windows: no WSL, no external imaging tools.
Prerequisites: [rustup](https://rustup.rs) (GNU host toolchain:
`rustup-init -y --default-host x86_64-pc-windows-gnu`, no Visual Studio
needed) and [QEMU](https://www.qemu.org/)
(`winget install SoftwareFreedomConservancy.QEMU`). Open a fresh terminal
after installing rustup so `cargo` is on PATH; the pinned toolchain and
`x86_64-unknown-none` target install automatically on first build via
`rust-toolchain.toml`.

```
cargo xtask build              # compile the kernel (x86_64-unknown-none)
cargo xtask image              # build target/wizard-os.img, a bootable UEFI disk image
cargo xtask run                # boot it in QEMU (serial output on stdio)
cargo xtask vmware             # generate the self-test VM in vm/self-test/
cargo xtask vmware --shipped   # generate the user-facing VM in vm/shipped/
cargo xtask help               # every task, including the check tasks below
```

`cargo xtask run -- <args>` passes arguments straight through to QEMU.
QEMU is found at `C:\Program Files\qemu` or on `PATH`; `QEMU_DIR` or `QEMU`
override that.

Kernel debugging additionally uses the pinned native GDB from WinLibs. The
batch command proves a clean setup without leaving a VM running; the ordinary
command opens an interactive prompt at the kernel's `kmain` entry, and quitting
GDB terminates its private QEMU machine.

```
winget install --id BrechtSanders.WinLibs.POSIX.UCRT --version 16.1.0-14.0.0-r4 --exact
cargo xtask debug --check
cargo xtask debug
```

Open a fresh terminal after the WinGet install, or set `GDB` to the exact
`gdb.exe`. The accepted debugger identity is GDB 17.2 from WinLibs r4 (D-231).

## Development

```
cargo xtask ci       # run the complete local gate below
```

`ci` is the ordinary local source gate; there is no hosted CI (D-26). Phase
close additionally requires the exceptional qualifications listed below.
`cargo xtask help` lists every task and renders this same registry. The marked
section is checked byte for byte against the registry that execution iterates,
so changing a Cargo graph or gate stage requires a reviewable documentation
change (D-212).

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
| 11 | `cargo xtask degraded-check --release` | prove the no-PCI and no-x2APIC refusals |
<!-- CI-REGISTRY:END -->

`boot-check` boots the shipped kernel and requires it to settle at its prompt;
`degraded-check` boots it with PCI configuration forced unavailable and on a
processor with no x2APIC. It requires both refusals, the hardware-free tmpfs and
devfs surveys, and a steady state rather than a triple fault.

Seven tasks are deliberately **not** in `ci`: `debug` needs the pinned native
GDB and is normally interactive, `model-check` needs TLC and Java, `vmware`
needs VMware Workstation to boot the result, `qualify-vmware` validates the
fresh transcript after that human boot, `mutate` needs one QEMU boot per known
defect, `baseline` needs six QEMU boots for descriptive measurements, and
`qualify-advisory` needs `cargo-audit` plus fresh RustSec data. A gate step that
fails for want of an exceptional tool or a long qualification teaches people
to skip the gate.
`model-check` prints the exact resolved Java identity before invoking it;
`vmware` prints the image kind, exact byte length and SHA-256 digest, resolved
`vmrun` identity and a fully quoted launch command when Workstation is
installed, or names that dependency as unavailable without blocking VM
generation. Self-test and shipped generation use separate machine state and
start with separate empty serial transcripts (D-218, D-235).
After booting each generated machine and completing its human input rounds, run
`cargo xtask qualify-vmware` for self-test and
`cargo xtask qualify-vmware --shipped` for the shipped image. Qualification
validates and preserves each
fresh transcript; generation alone is not phase-close execution evidence.

`cargo xtask baseline` builds the fixed-QEMU self-test image once in release
and debug modes, then boots each three times. It requires a clean ordinary
self-test verdict while enforcing reviewed bands for image size, unsafe count,
boot time, retained frame/heap/handle state, one real ring-3 trap round trip and
one full-buffer NVMe read. The reported variance is a regression alarm for this
emulated development machine, not a hardware benchmark or ABI promise (D-230).

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
`pc-q35-9.2`, not the moving `q35` alias. Evidence schema v6 requires
source-bound records for the current mutation sweep, the fresh RustSec review,
all model runs and both generated-and-booted VMware selectors, plus fixed-shape
Java and VMware tool identities. Verification therefore rejects an
unavailable, stale, omitted or changed phase-close qualification rather than
changing schema (D-218, D-244).

`test` runs the host suite; `test-boot` boots a kernel built with the
`selftest` feature headless in QEMU, injects a keystroke over QMP when the
kernel asks for one (D-38), and reads the verdict from the exit code, which
is how anything needing a real machine gets tested (D-28). `doc` exists to
make the rustdoc link lints fire: nothing else in the gate runs rustdoc, so
without it a doc link can rot unnoticed (D-22).

`api-audit` builds every registered target graph offline and requires an exact
compiler-resolved caller for each public `libs/` and `wost` item or enum
variant. Caller evidence is bound to its package, target, source origin and
compiler inputs; an unclassified source shape fails closed. Re-exports and
documentation do not count, and fields remain outside the claim because stable
rustc does not diagnose every live destructuring pattern (D-213, D-275).

Every Cargo-driven check runs over every graph in the registry above. A Cargo
feature is a different crate graph, and linting only one of them is how two
dead constants once rode a release image out. Adding a build-only feature
means adding its graph to `xtask/src/gate.rs`.

Every kernel build is read back and checked against what `kernel/linker.ld` is
written to produce: entry in the higher half, no orphan sections, no segment
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

`cargo xtask` is an alias as well. It runs the orchestrator under the optimized
`xtask` profile, so the tool hashes and parses the tree at full speed while
every graph it audits, every host test and the mutation workspace keep the
plain `dev` profile (D-292).

**Never set the `RUSTFLAGS` environment variable here.** It replaces the
`[target.x86_64-unknown-none]` rustflags in `.cargo/config.toml`, and the
kernel would link without `code-model=kernel`: a broken higher-half image
with no build error. Pass lint flags after `--` instead.

For the same reason, `.vscode/settings.json` points rust-analyzer at
`cargo xtask check`. Without it the editor checks the kernel for the host
triple and reports phantom errors across every kernel file.

## Layout

```
wost/       privileged kernel framework, the only crate that may write `unsafe`
kernel/     the kernel binary (no_std, #![deny(unsafe_code)], higher-half)
libs/       shared no_std crates: pure logic, host-testable (including
            memory-map, frame-space, ring-proto, and architecture encodings)
xtask/      build orchestrator: pure-Rust disk imaging, QEMU/VMware runners
boot/       limine.conf + vendored Limine UEFI binary (digest-pinned)
handoff/    synthetic route-neutral envelope/payload fixtures, not Wizzy source
docs/       design docs (design-first: docs before code)
models/     TLA+ ring-protocol specs, plus four deliberate mutant configs that
            must each fail, checked by `cargo xtask model-check` (D-112)
tools/      pinned TLA+ jar and the foreign FAT32 corpus, both digest-checked
supply-chain/  SBOM, third-party notices and license texts
userland/   native ABI header + the uncompiled mlibc pressure test from Phase 5
ports/      (future) recipes for mlibc, CPython, links, NetSurf
```
