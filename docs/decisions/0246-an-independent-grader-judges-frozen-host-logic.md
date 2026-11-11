# D-246 An independent grader judges frozen host logic

Status: Accepted, 2026-08-30. Extends D-28 and D-119; qualification tracked by PW-025.

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

**Rationale.** The project-authored mutation registry and the checks named by
it can share an assumption, and the last `cargo-mutants` experiment was an old
blocked research run, not a grade of the accumulated foundation.

**Evidence (2026-09-06).** `cargo-mutants 27.1.0` ran unmodified over every
host-testable library, each package in its own work tree, target directory
and output directory, unshuffled, with `--jobs 4 --timeout-multiplier 3`.
The first grade ran on `95c88f2`; its real gaps, such as `free_contiguous`
never seeing a run where D-105's ceiling must win, were closed by tests on
`c17dabd` and `46ff365`, with the complete grade rerun after each. The final
grade, on `46ff365`, tested 3,422 mutants across the 26 libraries: 3,072
caught, 90 missed, 248 unviable, 12 timeouts, with every miss an adjudicated
equivalent and no unadjudicated timeout, unviable mutant or build failure.
`xtask` is graded separately and its result is appended here when it exists.
The machine-readable outcomes and exact commands are archived under
`target/qualification/cargo-mutants/<commit>/`, which the evidence bundle does
not yet carry.

Equivalent mutants, by argument: `|` replaced by `^` between operands that
occupy disjoint bit positions, including where `^` binds tighter and joins two
disjoint constants the constructors keep disjoint; `>` replaced by `>=` where
both branches yield the clamp value at equality; `1 << 0` replaced by
`1 >> 0`; comparisons whose extra case feeds a `max` that already holds the
value or is refused by the following guard (`covered_by`, `first_fit_by`,
`first_memory_run`); `first_boundary` replaced by zero, which the page-run
iterator skips as an uncovered prefix; the `visited` counters in `read_file`,
whose bound the cycle hare reaches with the same `Unbounded`; `Bpb::parse`'s
active-FAT arms, which line 166 already forces to agree;
`valid_short_name_byte`'s space bound, which the scans never present;
`Entry::one_shot`'s zero timer-mode constant; the set-2 fake-shift guards,
whose fallback arm yields no event either; and the wide-BAR-at-last-register
check, since `probe_bars` refuses that window and no authority for it can
exist. The timeouts are all non-termination, for which the harness bound is the
only possible oracle. The unviable mutants are not programs: `Default` on types
that deliberately lack it, `Default::default` inside `const fn`, `std`
constructors in `no_std` crates, cast-then-compare text that parses as
generics, or const arithmetic the crates' compile-time ceilings refuse.
