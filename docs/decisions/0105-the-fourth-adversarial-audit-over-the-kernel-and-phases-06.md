# D-105 The fourth adversarial audit, over the kernel and Phases 0 to 6

Status: Accepted, 2026-08-24.

**Context.** D-77, D-94, D-95 and D-101 each attacked a slice. This one attacked
everything: `wost`, `kernel/src`, all nineteen `libs/` crates, and `xtask`
including the gate. Four passes ran in parallel, each told to treat every green
check as a claim and to reproduce rather than argue. The `libs`/`xtask` pass ran
sixty mutations and reported twelve survivors.

**Decision.** Fix what was found, and record the two rules the fixes taught
about which checks are worth having.

*Soundness, in `wost`.* Safe code reached undefined behaviour in two ways.
`map_boot` passed caller flags to `map_range` verbatim, and `Flags::OWNED` is
public in `libs/x86-paging`. A boot mapping could claim a frame it did not own,
and `unmap`, which consults that bit before freeing frames, would give the boot
handoff's memory a second owner. `map_boot`, `map_owned` and `unmap` now strip
`OWNED | HUGE` from caller flags. Separately, `AddressSpace::for_user` hands out
a space whose higher half **is** the live kernel's tables, and mapping calls
wrote through them: `u.unmap(kernel_stack, len)` blanked a running thread's page
table entries and freed its frames. A space now carries a `Reach`, and one that
shares the kernel refuses anything at or above `USER_END`.

*A widening that should have been a refusal.* `set` upgraded an interior table
to `USER` when a user leaf needed one. Its `SAFETY` comment read "this space is
still under exclusive construction", a premise D-86 deleted. An interior entry
covers everything below it, so the upgrade handed ring 3 every mapping under
that table. It is now a `Conflict`, at no cost: a user space's lower half starts
empty, so its tables are minted `User` on the way down. The refusal makes
`activate`'s claim true again, "a present entry accepts only an identical
rewrite", and lets the effective-permission fold move to `libs/`.

*Evidence that proved the wrong thing.* `BootRegion` meant "the memory map
describes this range", and a `Reserved` device hole is described. A writeback
mapping over one is a cacheable alias of registers. `describe` now requires
`MemoryKind::is_direct_mapped`, D-45's own answer to what counts as memory,
which is the map this evidence exists to build.

*A `#UD` a safe caller could arm.* `interrupt::arm_access_clear` is safe and
public and installs a `clac` in the trap tail. Without SMAP that is an invalid
opcode, and the fault handler re-enters the stub. It now asserts, as
`interrupt::enable` does; D-77 rejected a call-site check, which another safe
caller can omit.

*A read that could tear.* D-95 made the TSS stack writes atomic against a trap
arriving mid-write, and the readers undid it by copying the whole 104-byte
structure. `RSP0` moves on every switch, so a preempted reader could see half of
two stacks. It is one aligned eight-byte load now.

*A loader with no rollback.* `load_image`'s doc said "every segment is checked
before a page is taken"; the loop checked segment *N*, mapped it, then looked at
*N+1*. A later refusal left earlier segments mapped `USER` at addresses the
image chose, their frames ownerless, while the boot reported "could not load the
program". It is two passes now, with an overlap check, and the mapping pass
walks itself back as `map_range` does. `LoadError` gained `Truncated`,
`Overlapping` and `TooManySegments`, so one error names one mechanism; that let
the `contents_fit` bound be isolated at all.

*Defects outside the kernel.* `eevdf::Entity::charge` narrowed a widened product
with `as`. A charge whose virtual value is a multiple of `2^64` advanced the
clock by nothing, the outcome `MAX_WEIGHT`'s own doc says it exists to prevent.
It clamps now, as its two siblings always did. `xstate`'s `Layout::compacted`
truncated its cursor to `u32`, giving two components one offset, while
`is_consistent`, which the doc calls "the invariant a caller should assert
before handing the size to an allocator", was a constant `true`.
`xtask/src/elf.rs`, the verifier D-40 says must never panic on a malformed
input, panicked on two: `e_shoff = u64::MAX` and `p_filesz = u64::MAX`.
`handle_space::next_generation` panicked in the debug boot on a raw counter of
`u32::MAX`. And the shipped-boot check ignored the panic flag after its settle
window, so a panic in the final poll interval was reported as a clean boot.

*Two rules, from what the check failures had in common.*

**A check must not be satisfiable by the algebra of its own inputs.** The `libs`
pass found nine and reduced each. A stack layout proved
`base + stride·i ≤ base + stride·i`. An APIC crate spelled every rejection input
`FIRST_VECTOR - 1`, which panics for every `FIRST_VECTOR >= 1`. A Pause sequence
was checked byte-for-byte against itself: truncated to four bytes, it passed all
thirty-one tests, and the decoder emitted two phantom release events. A
reservation count was asserted equal to its own definition. The cheap repair is
always the same: **transcribe the number from the manual**. The crates that
survived the mutation pass (`acpi-tables`, `tss.rs`, `x86-apic/register.rs`,
`x86-pit/command.rs`, `sha256.rs`) already did.

**A check must fail for the reason it names, not merely fail.** `load_image`'s
overrun case tripped two refusals at once, so deleting the bound keeping a slice
index inside attacker-supplied bytes left the gate green. The `VERW` check read
the byte that *arms* the flush, not the instruction: the split behind D-101's
finding that the flush shipped unexecuted for a phase. It now reads a tally
incremented beside the `verw`. And `a_second_thread_crosses_onto_its_own_stack`
compared `segment::kernel_stack()` to itself, global CPU state read twice after
both crossings, instead of the stack each trap landed on.

*Where the arithmetic went.* The effective-permission fold left `wost` for
`Flags::through` / `Flags::effective_for` in `libs/x86-paging`. Its machine
check could not fail, and after the `set` refusal it *cannot*: no hierarchy this
kernel builds has a parent narrower than its child. A host test can hand it one.
The rule that pure arithmetic goes where `cargo test` reaches it was here the
fix.

*Four contracts written twice and checked nowhere.* The self-test check count,
the verdict version, the four serial boot lines and the ESP file names each live
in two files. All four were in step, but drift showed only in a sixty-second
boot or a forty-five-second image build, not in `cargo test`. `paths.rs` and
`qemu.rs` now read the sibling source and the `limine.conf`, and one counts
`report.check(` calls against `EXPECTED_CHECKS`.

**Consequences.** Twelve `libs/` crates, four `wost` modules, six `kernel`
modules and `xtask`'s ELF reader changed. Three new machine checks (108 now)
were each broken once and watched go red. So were the fold, the stack layout,
the gate encoding, the APIC constants, the Pause sequence, the mmio bound, the
pixel modulus and the charge clamp. `elf64` now refuses an ELF that is not
`EM_X86_64`; an aarch64 image had parsed cleanly and yielded segments.

Left owed: `xtask/src/elf.rs` still transcribes the ELF header offsets instead
of using `libs/elf64`, the duplication that crate exists to prevent.
`libs/elf64`'s module doc had claimed the merge and now says it has not.

~~`mm::with_memory_map` and `dev::with_io_apic` are still closures taken under a
leaf guard.~~ **Closed by D-121**: both are gone, replaced by queries that take
the lock, read what was asked, and give it back.

~~And a retired ring-3 thread still keeps its slot and its pages for the life of
the boot.~~ **Closed by D-117**: it becomes a zombie and is collected.
