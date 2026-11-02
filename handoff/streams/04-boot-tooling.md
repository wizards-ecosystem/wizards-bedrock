# Stream 4 — boot path and developer tooling

Scope: `boot/`, `xtask/`, `tools/`, `wost/`, and this note. Other streams own
the root workspace config, `kernel/`, and `libs/`.

## Done

- QEMU resolution no longer reports a missing emulator as missing UEFI
  firmware. `QEMU` and `QEMU_DIR` must name a real file before firmware search.
  Discovery names the Windows default, `PATH`, both variables, and the winget
  install command. Firmware search also covers an FHS prefix (`bin/` next to
  `share/qemu/`) and lists every path it tried, including the required edk2
  image names.
- A mistyped task within edit distance 2 names the nearest command. `run`
  flags that look like QEMU options say they belong after `--`.
- `cargo xtask help` names `QEMU`, `QEMU_DIR`, and `GDB`, and the two-edit
  suggestion window.
- A kernel build checks that the pinned toolchain actually contains `rustc`
  and `rustdoc` before cargo is spawned. The error names the missing file,
  `rust-toolchain.toml`, and `rustup show`.
- A missing `debug` debugger names the pinned WinLibs identity and says a
  different gdb is refused. A missing JVM for `model-check` says to put
  `java` on PATH and that `tools/tla/tla2tools.jar` is not a JVM.

## In progress

- Nothing. The next census pass is the leftover list below.

## Not finished

Measured with stream 1's `clippy.toml` (`CLIPPY_CONF_DIR=/tmp/bedrock-clippy`)
and `-W clippy::pedantic -W clippy::cast_possible_truncation`, not by editing
root config. After the cast helpers, self-test `wost` was about 118 unique
warnings. This round then documented MMIO, `pic::silence`, and
`port::release_spans`, and backticked the `doc_markdown` hits those runs named.
Still open, in that order:

- `# Errors` / `# Panics` on the remaining `wost` functions (paging, pci, vtd,
  interrupt, task, one boot enter).
- `cast_ptr_alignment` in `segment`, `mm`, `mmio`, `heap`, and paging. These
  are aligned by a prior check; an `expect` should name that check, not a
  silent cast.
- `needless_pass_by_value` and `fn_params_excessive_bools`. Several are
  kernel-facing signatures. An enum instead of `bool` needs a kernel edit,
  which this stream does not own.
- `exhaustive_enums`, `manual_let_else`, `unused_self`, `struct_excessive_bools`.
- `tools/` has no Rust. `boot/limine` was not changed. `wost/src/boot` only
  gained `usize_from_u64` for the framebuffer dimensions.
- `cargo::tests::kernel_builds_use_the_exact_codegen_and_virtual_path_contract`
  still fails on Linux because its `C:\` fixture contains `:`. Leave it until
  a Windows run; it is mutation 1789's witness.

## Next steps

- `wost/src/boot` handoff was read and left unchanged: the Limine glue already
  checks module length and address wrap before `from_raw_parts`, and the
  unsafe audit hashes every `wost` file. A later pass should only edit it
  for a defect a host test can fail.
- `cargo::tests::kernel_builds_use_the_exact_codegen_and_virtual_path_contract`
  uses `C:\...` fixtures and fails on Linux inside `join_paths` because `:`
  is the PATH separator. It is pre-existing and still the Windows witness
  for mutation 1789; do not retarget it without a Windows run.
- Full `cargo xtask ci`, QEMU boots, and the mutation campaign were not run.

## Bugs found

- A `QEMU` or `QEMU_DIR` value that was not a file was accepted. Firmware
  search then failed against that path's parent, so the error said the edk2
  image was missing.
- Firmware was sought only under the executable's directory. A `bin/` install
  with images in the prefix `share/qemu/` was reported missing even when the
  edk2 files were present.

## Decisions

- Do not accept `OVMF_CODE.fd` / `OVMF_VARS.fd` as substitutes. The runner is
  pinned to `edk2-x86_64-code.fd` and `edk2-i386-vars.fd`; the error names the
  difference instead of booting a different firmware image.
- No full `cargo xtask ci`, QEMU boot, or mutation campaign in this stream.
  Focused `cargo test -p xtask --lib` filters and `cargo check -p xtask` only.
