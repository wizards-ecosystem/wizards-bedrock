# Stream 06 — integration

Branch: `integration/bedrock-v0.3-prep`
Base: `origin/main` at `94bfcce` (Close the rows v0.2.0 proved).
Do not merge this branch into `main`. The pull request stays open for review.

## Done

- Review comments posted on drafts #6, #7, #5, #4, and #3. Comment, not approval.
- Merged those branches in that order. No conflicts.
- Cross-cutting signatures:
  - `establish_deny_all` takes `PlatformInterruptRemapping`.
  - `second_level::pte` takes `Permission`, built from
    `from_directions::<DEVICE_READS, DEVICE_WRITES>()`.
  - `map_device` takes `&AdmittedDeviceRange`.
  - `BarUnmapOutcome`, `BarEnableOutcome`, `Retargeting`, and `Establishment`
    are `#[non_exhaustive]`, with fail-closed `_` arms in the kernel.
- One-shot tokens stayed by value: `BootRegion`, `StackArena`, `Binding`,
  the bus-master proofs, and the cache-substitution test. `Protections`
  bool fields stayed.
- D-159 amended 2026-11-02. `cargo xtask clippy` still denies warnings.
  Truncation is per-site `try_from` or `#[expect]`.
- `require_unique`, the ACPI duplicate-table branch, and mutation 1819
  were not edited.
- Source contracts and unsafe fingerprints were retargeted onto the new
  text. Unsafe line and block counts did not change.

## Validation on Linux, rustc 1.97.1

Passed:

- `cargo xtask fmt --check`
- `cargo xtask clippy` (host, shipped kernel, self-test, refuse-pci)
- `cargo xtask doc`
- `cargo xtask supply-chain` (81 registry crates)
- `cargo xtask verify-vendor`
- `cargo xtask test-boot --release` and `--debug`:
  `selftest: verdict v1 PASS passed=294 failed=0 expected=294`
- `cargo xtask boot-check --release`
- `cargo xtask degraded-check --release`
- Library tests: `pci-config` 99, `memory-map` 26, `fat32` 42, `vtd` 37,
  `experiments/register-index` 4. All passed.
- `every_mutation_still_applies` passed. The mutation campaign
  (`cargo xtask mutate`) is not part of `ci` and was not run.

`cargo xtask ci` as one command stops earlier:

- `api-audit` fails because Linux activates the `libc 0.2.189` build
  script, which is not in the reviewed build-script set.
- `cargo xtask test` then reports 926 passed, 26 failed, 17 ignored.
  The 26 are the Windows codegen-path witness (mutation 1789), API
  fixture links that cannot find the host C libraries under `rust-lld`,
  two VMware symlink checks, and the security-deep tests that build
  Windows tool paths. Left unchanged.

## Next steps

None for this pass. Isaac reviews the pull request. Do not merge it.

## Decisions

- A by-value proof that is not `Copy` stays by value. The move is the spend.
- Neutral wording only for the decode and port-claim fixes.
- Isaac may override the warn-level pedantic rollout recorded in D-159.
