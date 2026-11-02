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

- Further kernel review. Waiter registration, scheduler/zombie accounting,
  NVMe completion matching, and loader placement were read and not changed.
  `WaitQueue::wait_unless` checks readiness while holding the waiter lock, so
  the classic lost-wakeup window is closed on this lock even though the prose
  says "register first".

## Next steps

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
