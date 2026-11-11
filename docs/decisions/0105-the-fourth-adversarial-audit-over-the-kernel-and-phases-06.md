# D-105 The fourth adversarial audit, over the kernel and Phases 0 to 6

Status: Accepted, 2026-08-24.

**Decision.** An audit over everything (`wost`, `kernel/src`, every `libs/`
crate, and `xtask` including the gate) fixes what it found and records two
rules about which checks are worth having.

**A check must not be satisfiable by the algebra of its own inputs.** A stack
layout proved `base + stride·i ≤ base + stride·i`; a Pause sequence checked
against itself passed truncated to four bytes. The cheap repair is always the
same: **transcribe the number from the manual**.

**A check must fail for the reason it names, not merely fail.** A case that
trips two refusals at once lets either be deleted with the gate green. The
`VERW` check reads a tally incremented beside the `verw`, not the byte that
*arms* the flush.

The fixes:

- **`map_boot`, `map_owned` and `unmap` strip `OWNED | HUGE` from caller
  flags**, because `Flags::OWNED` is public and a boot mapping could otherwise
  claim a frame it did not own.
- **A space carries a `Reach`**, and one that shares the kernel refuses anything
  at or above `USER_END`, because `for_user`'s higher half **is** the live
  kernel's tables.
- **`set` refuses to upgrade an interior table to `USER`** (`Conflict`): an
  interior entry covers everything below it, so the upgrade handed ring 3 every
  mapping under that table. A user space's lower half starts empty, so its
  tables are minted `User` on the way down. This makes "a present entry accepts
  only an identical rewrite" true again.
- **`BootRegion::describe` requires `MemoryKind::is_direct_mapped`**, D-45's own
  answer to what counts as memory, because a `Reserved` device hole is
  described and a writeback mapping over one is a cacheable alias of registers.
- **`interrupt::arm_access_clear` asserts SMAP**, as `interrupt::enable` does;
  without SMAP its `clac` is an invalid opcode. D-77 rejected a call-site
  check, which another safe caller can omit.
- **TSS readers do one aligned eight-byte load**, not a copy of the whole
  structure, since `RSP0` moves on every switch.
- **`load_image` is two passes with an overlap check**, and the mapping pass
  walks itself back as `map_range` does. `LoadError` has `Truncated`,
  `Overlapping` and `TooManySegments`, so one error names one mechanism.
- `eevdf::Entity::charge` clamps instead of narrowing with `as`, as its two
  siblings do. `xstate`'s `Layout::compacted` no longer truncates its cursor
  to `u32`, and `is_consistent` is no longer a constant `true`.
  `handle_space::next_generation` does not panic on a raw counter of
  `u32::MAX`. `elf64` refuses an ELF that is not `EM_X86_64`.
  `xtask/src/elf.rs`, which D-40 says must never panic on a malformed input,
  checks `e_shoff` and `p_filesz` before adding. The shipped-boot check reads
  the panic flag through the final poll interval.
- The effective-permission fold lives in `libs/x86-paging` as `Flags::through` /
  `Flags::effective_for`, where a host test can hand it a hierarchy no kernel
  build has.
- **Four contracts written twice are checked by `cargo test`**: the self-test
  check count, the verdict version, the four serial boot lines and the ESP file
  names. `paths.rs` and `qemu.rs` read the sibling source and the
  `limine.conf`, and one counts `report.check(` calls against
  `EXPECTED_CHECKS`.

**Rationale.** Every surviving defect was a check that restated its input or
a contract written in two places; both rules make a check say something the
code did not already say.

**Consequences.** Left owed: `xtask/src/elf.rs` still transcribes the ELF header
offsets instead of using `libs/elf64`, the duplication that crate exists to
prevent.

~~`mm::with_memory_map` and `dev::with_io_apic` are still closures taken under a
leaf guard.~~ **Closed by D-121**: both are gone, replaced by queries that take
the lock, read what was asked, and give it back.

~~And a retired ring-3 thread still keeps its slot and its pages for the life of
the boot.~~ **Closed by D-117**: it becomes a zombie and is collected.
