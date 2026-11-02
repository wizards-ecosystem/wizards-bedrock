# Stream 2 — kernel/

## Done

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

- Further kernel review: waiter registration, scheduler/zombie accounting,
  NVMe completion matching, and loader placement were read and not changed.
  `WaitQueue::wait_unless` checks readiness while holding the waiter lock, so
  the classic lost-wakeup window is closed on this lock even though the prose
  says "register first".

## Next steps

- Error enums that already implement `Display` but not `core::error::Error`
  (`EarlyTableError`, `InterruptEnableError`, `RoutingError`, `TakeError`,
  `ClaimError`, `LoadError`, `user::SpawnError`, `KeyboardRouteError`, and any
  siblings found the same way). `VfsError` also lacks `Display`.
- `acpi::find_table_with_multiplicity`: the non-unique arm returns on the
  first match, so the later `found.is_some()` / `return found` path for
  `require_unique == false` is dead.
- `Queue::abandon` posts completions and bumps `served` but does not
  `wake_all`, unlike `expire`. Production collect runs after the user thread
  is gone; only worth changing if a live waiter can observe abandon.
- Do not add a new `report.check` without also updating `EXPECTED_CHECKS` and
  the xtask copies (`xtask/src/qemu.rs`, `xtask/src/vmware.rs`). Those files
  are outside this stream.

## Bugs found

- Medium: PCI worklist duplicate secondaries hide a later bus. Fixed in
  `kernel/src/pci.rs`.

## Decisions

- Behavior change is limited to broken or repeated bridge secondaries. A bus
  number is queued once; a distinct later secondary is no longer dropped when
  the worklist would have been full of duplicates.
- No new machine-check slot. The pure worklist proof is AND-ed into the
  existing bridge-descent check.
- No edits outside `kernel/` except this handoff file.
