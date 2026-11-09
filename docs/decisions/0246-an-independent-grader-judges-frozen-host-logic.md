# D-246 An independent grader judges frozen host logic

Status: Accepted, 2026-08-30. Extends D-28 and D-119; qualification tracked by PW-025.

**Context.** The project-authored mutation registry and the checks named by it
can share an assumption. Its last `cargo-mutants` experiment was an old blocked
research run, not a grade of the accumulated foundation.

**Decision.** Run unmodified `cargo-mutants` 27.1.0 against every host-testable
workspace package from one clean source-frozen commit. Each package gets an
isolated work tree, target directory and machine-readable output. Project code
does not choose or rewrite the grader's mutants. Every survivor, timeout,
unviable mutant and build failure is adjudicated; an actionable result gets a
test or fix, followed by a complete current-source rerun. Equivalence requires
a source-level argument, not merely a familiar-looking mutant.

The grade records the commit, exact commands, package coverage, per-package
and aggregate totals, and every adjudication. It passes only with no
unadjudicated miss, timeout, unviable mutant or build failure.

**Evidence.** This entry fixes the method without inventing the result. PW-025
remains open until the frozen run is complete; its factual totals and artifact
locations will be appended here before phase close.


**Evidence (2026-09-06).** `cargo-mutants 27.1.0` ran unmodified over every
host-testable library, each package in its own work tree, target directory
and output directory, unshuffled, with `--jobs 4 --timeout-multiplier 3`.
The first grade, on `95c88f2`, tested 3,447 mutants across the 26 libraries:
3,039 caught, 147 missed, 249 unviable, 12 timeouts. Every non-caught outcome
was adjudicated; the gaps were closed on `c17dabd` and `46ff365`, and the
complete grade was rerun after each. The final grade, on `46ff365`, tested
3,422 mutants: 3,072 caught, 90 missed, 248
unviable, 12 timeouts, with every miss an adjudicated equivalent
and no unadjudicated timeout, unviable mutant or build failure. `xtask`
itself, about 3,460 mutants that each rebuild and run its 622-test suite, is
graded separately; its result is appended here when it exists. The
machine-readable outcomes of every package, with the exact commands, are
archived beside the other qualification records under
`target/qualification/cargo-mutants/<commit>/`, which the evidence bundle does
not yet carry.

Real gaps, each closed by a test: the NVMe admin command layouts
(`number_of_queues`, both queue creations and the PRP high halves) had only
been exercised with counts of one, queue one and addresses below 4 GiB, where
every shifted field is zero; the 8.3 decoder's one fixture used an `ARCHIVE`
attribute byte equal to a space, so an eight-byte base, a three-byte
extension under another attribute and an invalid byte in the extension's
second position were never decoded; `first_fit_by` never saw a zero length or
a non-power-of-two alignment; `free_contiguous` never saw a run leaving the
space while an earlier frame was already free, where D-105's ceiling must win;
`reap_up_to` never claimed exactly the posted head; the add-only
`Cr4Additions` doors were never applied to a register already holding their
bits; `BarAuthority::index`, `PhysicalAddressWidth::bits`,
`DeviceRange::physical` and `SharedLayout::bytes` were read only by `wost` or
the kernel; a lease the BAR authority was not bound to, an unmapped BAR
index, a retired command phase, a second memory-space admission and a command
readback that differs after probe restoration were never presented to
`pci-config`; and nine `Display` bodies had no oracle. Three simplifications
fell out: `copy_rows_up` lost a span guard that the constructor's
`span = pitch × height` and `width × 4 <= pitch` already prove unreachable
once rows are clipped to the height; `decode_slot` lost a dot-entry check no
decoded name can reach, because `.` is an invalid short-name byte; and the
NVMe buffer setter lost the PRP2 parameter every caller passed as zero.

Equivalent mutants, by argument: `|` replaced by `^` between operands that
occupy disjoint bit positions, including the cases where `^` binds tighter
than `|` and so joins two disjoint constants (descriptor, control-word,
rights, flag, slot, command and address fields whose disjointness the
constructors enforce); `>` replaced by `>=` where both branches yield the
clamp value at equality; `1 << 0` replaced by `1 >> 0`; comparisons whose
extra case feeds a `max` that already holds the value or is refused by the
following guard (`covered_by`, `first_fit_by`, `first_memory_run`);
`first_boundary` replaced by zero, which the page-run iterator skips as an
uncovered prefix; the `visited` counters in `read_file`, whose bound the
cycle hare reaches with the same `Unbounded`; `Bpb::parse`'s active-FAT arms,
which line 166 already forces to agree; `valid_short_name_byte`'s space
bound, which the scans never present; `Entry::one_shot`'s zero timer-mode
constant; the set-2 fake-shift guards, whose fallback arm yields no event
either; and the wide-BAR-at-last-register check, since `probe_bars` refuses
that window and no authority for it can exist.

Timeouts are all non-termination: loop indices whose `+=` became `*=` and
boundary walks that stopped being strictly increasing, for which the harness
bound is the only possible oracle. Unviable mutants are not programs: the
grader asked for `Default` on typed proofs and classes that deliberately lack
it, called `Default::default` inside `const fn`, invented `Range`, `Vec`,
`Box` and `std::iter` constructors in `no_std` crates, produced
cast-then-compare text that parses as generics, or changed const arithmetic
that the crates' own compile-time ceilings refuse. One ring-proto entry in
the first grade was a mingw linker collision, not a mutant property; the
reruns graded it caught.
