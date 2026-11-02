# Stream 06 — integration

Branch: `integration/bedrock-v0.3-prep`
Base: `origin/main` at `94bfcce` (Close the rows v0.2.0 proved).
Do not merge this branch into `main`. The integration pull request stays open for review.

## Done

- Review comments posted on the five drafts. Comment, not approval.
  - #6 lint foundation
  - #7 libs
  - #5 kernel
  - #4 boot tooling
  - #3 roadmap
- Merged, in order, with merge commits and no conflicts:
  1. #6 `cursor/beautiful-rust-lints-b7b1`
  2. #7 `cursor/libs-soundness-and-lints-8fd3`
  3. #5 `cursor/kernel-review-e5bf`
  4. #4 `cursor/boot-tooling-dx-091e`
  5. #3 `cursor/roadmap-v030-docs-bb4c`
- Cross-cutting signatures that are facts, not tokens:
  - `Vtd::establish_deny_all` takes `PlatformInterruptRemapping::{Present, Absent}`.
    Kernel passes `Present`. Mutation 1707 follows `is_present()`.
  - `second_level::pte` takes `Permission`. The mapper builds it with
    `Permission::from_directions::<DEVICE_READS, DEVICE_WRITES>()`.
    Mutation 1802 still publishes `Permission::ReadWrite` first.
  - `AddressSpace::map_device` takes `&AdmittedDeviceRange`.
  - `BarUnmapOutcome`, `BarEnableOutcome`, `Retargeting`, and `Establishment`
    are `#[non_exhaustive]`. Kernel matches end in a fail-closed `_` arm.
- One-shot tokens stayed by value. The move is the spend, and a reference
  would let the caller reuse it:
  - `BootRegion` on `map_boot` (the type says it is not `Copy` for this reason)
  - `StackArena` on `unmap_stacks` (success drops the identities; failure
    returns them inside `StackRollback`)
  - `Binding` on `release_uninstalled` (neither `Copy` nor `Clone`)
  - `NvmeBusMasterAuthority`, `DmaDomain`, and `InterruptIsolation` on
    `admit_bus_master`
  - `DeviceMapping::rejects_cache_substitution_for_test` returns `Self`
- `cpu::Protections` bool fields stayed. Kernel still reads `.nx` and
  `.write_protect`.
- D-159 amended 2026-11-02: pedantic and `clippy::all` are warn,
  `cargo xtask clippy` still passes `-D warnings`, truncation stays warn
  and is answered per site, indexing and nursery stay off, sign-loss and
  wrap stay allow. `docs/style/BEAUTIFUL_RUST.md` matches that.
- `require_unique`, the ACPI duplicate-table branch, and mutation 1819
  were not edited.
- Mutation anchors and DMA source contracts were retargeted where a split
  or a cast helper moved the text. The defect each mutation inserts is the
  same one. Two NVMe DMA calls were taken back out of `assert!` so the
  contract can see them; `manual_assert` is expected there for that reason.
- Host clippy (`cargo clippy --all-targets --locked -- -D warnings`) passed.
  Shipped, self-test, and `refuse-pci` `cargo kclippy -- -D warnings` passed.
  `every_mutation_still_applies` passed. `vtd` library tests passed
  (37). Decision-log citation tests passed.

## In progress

`cargo xtask ci` has not been run yet. QEMU has not been booted yet.

## Next steps

1. Run `cargo xtask ci`. Fix failures caused by the merge.
2. If QEMU is installed, run the repository's boot step. The Windows-only
   codegen-path test (mutation 1789) is expected to fail on Linux. Do not
   change it.
3. Skip the long mutation campaign unless `ci` includes it and it finishes
   in reasonable time.
4. Open the pull request to `main`. Do not merge it.

## Decisions

- Review comments describe incorrect decode and wrong port claims in
  neutral wording. No vulnerability write-up.
- Isaac's lead, recorded so a resume does not relitigate it:
  `cargo xtask clippy` keeps `-D warnings`. Truncation is answered per
  site. D-159 is amended rather than replaced. Isaac may override.
- A by-value proof that is not `Copy` stays by value even when the body
  only reads the fields. Clippy's `needless_pass_by_value` does not see
  that the move is the capability.
