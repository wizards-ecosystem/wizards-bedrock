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

- Nothing.

## Kernel-facing signatures left for the integration pass

Do not change these in `wost` alone. Each one is still the current signature.
Clippy is silenced on the `wost` side with `#[expect]` and the reason names
the kernel caller.

### Bool parameter

- Item: `wost::arch::x86_64::vtd::Vtd::establish_deny_all(registers: ReleasableDevice, platform_interrupt_remapping: bool) -> Establishment`
- Caller: `kernel/src/iommu.rs` passes `true`.
- Suggested: `enum PlatformInterruptRemapping { Present, Absent }` and call `establish_deny_all(registers, PlatformInterruptRemapping::Present)`.

Private VT-d toggles stay `bool` on purpose. Their call sites and mutations
spell `true` or `false`. Suggested splits, still inside `wost` only, if a
later pass wants them: `enable_queued_invalidation` / `disable_queued_invalidation`,
the same pair for interrupt remapping and for translation, and
`invalidate_context` / `invalidate_iotlb` instead of `invalidate(&mut self, context: bool)`.

### `needless_pass_by_value`

Clippy says the body does not consume the value. A reference matches that.
The move is what the kernel call sites pass today.

- `AddressSpace::map_boot(&mut self, virtual_base: u64, region: BootRegion, flags: Flags) -> Result<(), MapError>`
  - Callers: `kernel/src/mem.rs` (`space.map_boot(...)`).
  - Suggested: `region: &BootRegion`.
- `AddressSpace::map_device(&mut self, admitted: AdmittedDeviceRange) -> Result<DeviceMapping, MapError>`
  - Callers: `kernel/src/mem.rs`, `kernel/src/selftest.rs`.
  - Suggested: `admitted: &AdmittedDeviceRange`.
- `AddressSpace::unmap_stacks(&mut self, arena: StackArena) -> Result<usize, StackRollback>`
  - Caller: `kernel/src/mem.rs`.
  - Suggested: `arena: &StackArena`.
- `pci::admit_bus_master(authority: NvmeBusMasterAuthority<'_>, domain: DmaDomain<'_>, interrupts: InterruptIsolation<'_>, state: &mut CommandState) -> Result<(), EnableError>`
  - Caller: `kernel/src/pci.rs`.
  - Suggested: `authority: &NvmeBusMasterAuthority<'_>`, `domain: &DmaDomain<'_>`, `interrupts: &InterruptIsolation<'_>`.
- `task::release_uninstalled(binding: Binding) -> Released`
  - Callers: `kernel/src/user.rs`, `kernel/src/thread.rs`, `kernel/src/selftest.rs`.
  - Suggested: `binding: &Binding`. The function's comment says the move spends the one-shot token; if `Binding` is `Copy`, the move does not do that.
- `DeviceMapping::rejects_cache_substitution_for_test(self, page: usize) -> (Self, bool)`
  - Caller: `kernel/src/selftest.rs` (`mapping.rejects_cache_substitution_for_test(1)`).
  - Suggested: keep `self` if the method must hand the mapping back. `&self` changes the return.

### `exhaustive_enums` that kernel matches without `_`

- `paging::BarUnmapOutcome` — `kernel/src/mem.rs`.
- `pci::BarEnableOutcome` — `kernel/src/pci.rs`.
- `vtd::Retargeting` and `vtd::Establishment` — `kernel/src/iommu.rs`.

`TableWalks` and `SpinLockWitness` are `#[non_exhaustive]` already. Kernel
compares `SpinLockWitness` with `==`, which does not need a wildcard.

### `struct_excessive_bools` read by kernel

- `cpu::Protections` fields `write_protect`, `smep`, `smap`, `umip`, `nx`.
  Callers: `kernel/src/user.rs`, `kernel/src/selftest.rs` (`Protections::enabled().nx` and `.write_protect`).
  Suggested: leave the fields until a kernel change can group them.

## Not finished

`cargo::tests::kernel_builds_use_the_exact_codegen_and_virtual_path_contract`
still fails on Linux because its `C:\` fixture contains `:`. Left alone. It is
mutation 1789's Windows witness.

Other pedantic `wost` warnings that were not in this round's list, still
present under the stream 1 table: `borrow_as_ptr` (apic, xstate),
`elidable_lifetime_names` (apic), `manual_assert` (paging, boot/mod),
`nonminimal_bool` (paging), `single_match_else` (paging, sync),
`similar_names` (apic), `match_same_arms` (apic), `semicolon_if_nothing_returned`
(cpu), `too_many_lines` (paging `map_owned`-sized function), `map_unwrap_or`
(task), `if_not_else` (task), `used_underscore_binding` (pci),
`items_after_statements` and `checked_conversions` (limine glue).

`tools/` has no Rust. Full `cargo xtask ci`, QEMU, and the mutation campaign
were not run.

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
