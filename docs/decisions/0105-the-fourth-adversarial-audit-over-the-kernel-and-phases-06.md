# D-105 The fourth adversarial audit, over the kernel and Phases 0–6

Status: Accepted, 2026-08-24.

**Context.** D-77, D-94, D-95 and D-101 each attacked a slice. This one was
asked to attack everything: `wost`, `kernel/src`, all nineteen `libs/` crates,
and `xtask` including the gate itself. Four passes ran in parallel, each told
to treat every green check as a claim and to reproduce rather than argue —
the `libs`/`xtask` pass ran sixty mutations and reported twelve survivors.

**Decision.** Fix what was found, and record the two things the fixes changed
about how this repository decides a check is worth having.

*Soundness, in `wost`.* Two ways safe code reached undefined behaviour.
`map_boot` passed caller flags to `map_range` verbatim, and `Flags::OWNED` is
public in `libs/x86-paging` — so safe kernel code could mint a boot mapping
claiming to own a frame it did not, and `unmap`, which consults exactly that
bit before handing frames to the allocator, would give the boot handoff's
memory a second owner. `map_boot`, `map_owned` and `unmap` now strip
`OWNED | HUGE` from what a caller supplies. Separately, `AddressSpace::for_user`
hands out a space whose higher half **is** the live kernel's tables, and every
mapping call wrote through them: `u.unmap(kernel_stack, len)` blanked a running
thread's page table entries and freed its frames. A space now carries a `Reach`,
and one that shares the kernel refuses anything at or above `USER_END`.

*A widening that should have been a refusal.* `set` upgraded an interior table
to `USER` when a user leaf needed one, under a `SAFETY` comment reading "this
space is still under exclusive construction" — a premise D-86 deleted. An
interior entry covers everything below it, so the upgrade handed ring 3 every
mapping already under that table. It is now a `Conflict`, which costs nothing:
a user space's lower half starts empty, so its tables are minted `User` on the
way down. That refusal makes `activate`'s standing claim — "a present entry
accepts only an identical rewrite" — true again, and it is what lets the
effective-permission fold move to `libs/`.

*Evidence that proved the wrong thing.* `BootRegion` meant "the memory map
describes this range", and a `Reserved` device hole is described. A writeback
mapping over one is a cacheable alias of registers. `describe` now requires
`MemoryKind::is_direct_mapped` — D-45's own answer to what counts as memory,
which is the map this evidence exists to build.

*A `#UD` a safe caller could arm.* `interrupt::arm_access_clear` is safe and
public and installs a `clac` in the trap tail; without SMAP that is an invalid
opcode, and the fault handler re-enters the stub that executed it. It asserts
now, like `interrupt::enable` does. A call-site check is what D-77 rejected:
another safe caller can omit it.

*A read that could tear.* D-95 made the TSS stack writes atomic against a trap
arriving mid-write, and the readers undid it on the far side by copying the
whole 104-byte structure. `RSP0` moves on every context switch, so a preempted
reader could see half of one stack and half of another. One aligned eight-byte
load instead.

*A loader with no rollback.* `load_image`'s doc said "every segment is checked
before a page is taken"; the loop checked segment *N*, mapped it, then looked
at *N+1*. Any refusal after the first mapping left earlier segments mapped
`USER` at addresses the image chose, with their frames forgotten into the space
and no owner — while the boot reported "could not load the program". It is two
passes now, with an overlap check validation could not otherwise make, and the
mapping pass walks itself back the way `map_range` does. `LoadError` gained
`Truncated`, `Overlapping` and `TooManySegments` so that one error names one
mechanism — which is what let the `contents_fit` bound be isolated at all.

*Real defects outside the kernel.* `eevdf::Entity::charge` narrowed a widened
product with `as`, so a charge whose virtual value is a multiple of `2^64`
advanced the clock by nothing — the outcome `MAX_WEIGHT`'s own doc says it
exists to prevent, reached by the other door. It clamps now, like its two
siblings always did. `xstate`'s `Layout::compacted` truncated its cursor to
`u32`, silently giving two components one offset, while `is_consistent` —
which the doc calls "the invariant a caller should assert before handing the
size to an allocator" — was a constant `true`. `xtask/src/elf.rs`, the verifier
D-40 says must never panic on a malformed input, panicked on two:
`e_shoff = u64::MAX` and `p_filesz = u64::MAX`. `handle_space::next_generation`
panicked in the debug boot on a raw counter of `u32::MAX`. And the shipped-boot
check stopped consulting the panic flag after its settle window, so a panic
printed in the final poll interval was forwarded to the log and then reported
as a clean boot.

*Two rules, from what the check failures had in common.*

**A check must not be satisfiable by the algebra of its own inputs.** The
`libs` pass found nine of these and gave the reduction for each — a stack
layout proving `base + stride·i ≤ base + stride·i`, an APIC crate whose every
rejection test spelled its input `FIRST_VECTOR - 1` (which panics for every
`FIRST_VECTOR >= 1`), a Pause sequence checked byte-for-byte against itself
(the constant truncated to four bytes passed all thirty-one tests, and the
decoder then emitted two phantom release events), a reservation count asserted
equal to an expression that defines it. The repair is always the same and it is
always cheap: **transcribe the number from the manual**. The crates that
survived the mutation pass — `acpi-tables`, `tss.rs`, `x86-apic/register.rs`,
`x86-pit/command.rs`, `sha256.rs` — are exactly the ones that already did.

**A check must fail for the reason it names, not merely fail.** `load_image`'s
overrun case tripped two refusals at once, so deleting the bound that keeps a
slice index inside attacker-supplied bytes left the gate green. The `VERW`
check read the byte that *arms* the flush rather than the instruction, which is
precisely the split that let D-101 find the flush had shipped unexecuted for a
phase; it now reads a tally incremented beside the `verw`. And
`a_second_thread_crosses_onto_its_own_stack` compared `segment::kernel_stack()`
to itself — global CPU state read twice, long after both crossings — where the
property is the stack each trap actually landed on.

*Where the arithmetic went.* The effective-permission fold left `wost` for
`Flags::through` / `Flags::effective_for` in `libs/x86-paging`. Its machine
check could not fail, and after the `set` refusal above it *cannot*: no
hierarchy this kernel builds has a parent narrower than its child. On the host
a test can hand it one. This is the project's own rule — pure arithmetic goes
where `cargo test` reaches it — arriving at a case where the rule was the fix.

*Four contracts that were written twice and checked nowhere.* The self-test
check count, the verdict version, the four serial markers and the ESP file
names each live in two files. All four were in step, and a drift in any is
found by a sixty-second boot or a forty-five-second image build rather than by
`cargo test`. `paths.rs` and `qemu.rs` now read the sibling source and the
`limine.conf`, and one of them counts `report.check(` calls against
`EXPECTED_CHECKS`.

**Consequences.** Twelve `libs/` crates, four `wost` modules, six `kernel`
modules and `xtask`'s ELF reader changed. Three new machine checks (108 now),
each broken once and watched go red, as were the fold, the stack layout, the
gate encoding, the APIC constants, the Pause sequence, the mmio bound, the
pixel modulus and the charge clamp. `elf64` now refuses an ELF that is not
`EM_X86_64` — an aarch64 image parsed cleanly and yielded segments.

Left owed, and named rather than quietly carried: `xtask/src/elf.rs` still
carries its own transcription of the ELF header offsets rather than using
`libs/elf64`, which is the duplication that crate exists to prevent —
`libs/elf64`'s module doc claimed the merge had happened and now says it has
not. ~~`mm::with_memory_map` and `dev::with_io_apic` are still closures taken
under a leaf guard.~~ **Closed by D-121**: both are gone, replaced by queries
that take the lock, read what was asked, and give it back. ~~And a retired
ring-3 thread still keeps its slot and its pages for the life of the boot.~~
**Closed by D-117**: it becomes a zombie and is collected.
