# Stream 2 — kernel/

## Done

- 8042 output flush checks status after the last discarded byte. The old
  `.any` loop treated "the 32nd read emptied the buffer" as failure, so a
  controller that drained on the final allowed read was reported absent.
- PCI enumeration queues each bus number at most once. A bridge secondary that
  was already queued (but not yet visited) used to take another worklist slot.
  Two hundred and fifty-five copies of one secondary filled the 256-entry list
  while bus 0 was still being scanned, so a later distinct secondary on that
  bus was never visited and storage behind it was invisible. Correct topologies
  (one slot per bus) are unchanged. The proof is
  `pci::duplicate_secondaries_do_not_hide_a_later_bus_for_test`, folded into
  the existing machine check `the bus walk descends through a bridge` so
  `EXPECTED_CHECKS` stays 294. xtask's mutation 212 still matches the bridge
  `if` in `kernel/src/pci.rs`.

## In progress

- None. Shipped and `selftest` kernel graphs are clean against stream 1's
  clippy table. Checked with `CLIPPY_CONF_DIR` pointed at a copy of
  `origin/cursor/beautiful-rust-lints-b7b1:clippy.toml` and that branch's
  warn/allow set passed after `--`. Root `Cargo.toml` was not edited.

## Unsafe audit

- `kernel/` has `#![deny(unsafe_code)]` and no hand-written `unsafe` block.
  The only unsafe attributes are inside `wost::entry!`, which expands
  `#[allow(unsafe_code)]`, `#[unsafe(link_section)]`, and `#[unsafe(no_mangle)]`
  into `kernel/src/main.rs`. Those attributes and their contracts live in
  `wost/src/boot/mod.rs` (stream 4). There is no kernel `SAFETY` comment to add.

## Next steps

- Turn `selftest.rs` module `expect`s into per-site `try_from` if the fixture
  casts should stay visible to the lint.
- Replace `expect(too_many_lines)` procedures (`start_interrupts`, `build`,
  `run_on`, `load_image`, `spawn_transaction`, `collect`, `spawn_prepared`)
  with real splits only if a reader can still see the order.
- `data_mapped: bool` and `require_unique: bool` are still bools. The ACPI
  one is pinned by mutation 1819.
- `Queue::abandon` posts completions and bumps `served` but does not
  `wake_all`, unlike `expire`. Production collect runs after the user thread
  is gone; only worth changing if a live waiter can observe abandon.
- `acpi::find_table_with_multiplicity` looks like it has a dead non-unique
  branch, but mutation 1819 in `xtask/src/mutate.rs` matches the
  `if require_unique` text exactly. Do not delete or rewrap that branch in
  this stream.
- Do not add a new `report.check` without also updating `EXPECTED_CHECKS` and
  the xtask copies (`xtask/src/qemu.rs`, `xtask/src/vmware.rs`). Those files
  are outside this stream.

## Bugs found

- Medium: PCI worklist duplicate secondaries hide a later bus. Fixed in
  `kernel/src/pci.rs`.
- Low: `i8042::Controller::flush` reported failure when the 32nd discarded
  byte was the one that emptied the output buffer. Fixed in
  `kernel/src/arch/x86_64/i8042.rs`. A one-byte controller buffer never hits
  this; a backlog of exactly `FLUSH_LIMIT` bytes did.

## Clippy

- Shipped graph: truncating casts are `try_from` where the value must fit,
  and `#[expect(clippy::cast_possible_truncation, reason = ...)]` where the
  cast is a register half (PCI command/vendor, NVMe address dword, PIT window
  that the const assertion already bounds, page-size constants).
- Other shipped warns were fixed (docs, `assert!`, `if let`, build-script
  `expect`) or marked with `expect` and a reason where the function is one
  ordered procedure (`too_many_lines`, invariant `panic` inside `Result`).
- `selftest` graph: the same, plus module-level `expect`s on
  `kernel/src/selftest.rs` for fixture casts and harness shape. Those expects
  are the follow-up if someone wants each fixture cast to be `try_from`.

## Cleanups

- `core::error::Error` for kernel errors that already had `Display` and
  `Debug`: `InterruptEnableError`, `DmaFenceError`, `ClaimError`,
  `BarAdmissionError`, `ReleaseError`, `EarlyTableError`, `RoutingError`,
  `TakeError`, `Refusal`, `TimePageError`, `LoadError`, `user::SpawnError`,
  `KeyboardRouteError`, `BuildError`, `BusMasterAdmissionError`,
  `BringUpError`. `VfsError` gained `Display` and `Error`. No `source()`
  chains; nested types were not all checked for `Error`.

## Decisions

- Behavior change is limited to broken or repeated bridge secondaries. A bus
  number is queued once; a distinct later secondary is no longer dropped when
  the worklist would have been full of duplicates.
- No new machine-check slot. The pure worklist proof is AND-ed into the
  existing bridge-descent check.
- No edits outside `kernel/` except this handoff file.
