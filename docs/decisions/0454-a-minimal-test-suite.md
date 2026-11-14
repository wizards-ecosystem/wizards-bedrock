# D-454 A minimal test suite

Status: Accepted, 2026-11-14. Builds on D-453 item 5. Supersedes the test and mutation mandates of D-28, D-119, D-134, D-210, D-301, D-322 and D-341, and the release gate of D-425. Ends D-441's qualified-diagnostic-release contract. Removes the machinery mandated by every decision that introduced it, among them D-213, D-246, D-275, D-330, D-373 and D-406. Amends D-451.

**Context.** The tree held 1,965 host tests, 296 machine checks and a 2,450-entry
mutation registry. About 116,000 lines were test or test machinery against
82,000 of product, and `xtask` was mostly gate and qualification tooling. Most
of that suite restated the code: documentation pins, source-text contract
scans, a hashed table of every `unsafe` block and a registry whose job was to
keep the suite honest. Each deleted or renamed check also moved ids, counts and
hashes in several files, so the suite taxed every change more than it caught.
Linux, seL4, Fuchsia, Hubris and Cloud Hypervisor rely on a boot check, a
syscall-level suite, fuzzing and per-bug regressions; source rules live in
linters, and none of them keeps a hand-maintained mutation list.

**Decision.** The suite has six tiers and nothing else.

1. **Boot smoke.** One boot reaches the shell and settles, and a scan of its
   serial log finds no warning or panic, not only the final verdict.
2. **Syscall and ABI behaviour.** Self-test checks that cross the real call
   boundary: valid calls, documented refusals, bad pointers, handle rights,
   rings, faults and process exit.
3. **Isolation and security.** A program cannot reach another's memory or the
   kernel's, cannot use a revoked or wrong handle and cannot DMA outside its
   domain; kernel memory is cleared on reuse; SMAP, NX and KPTI hold.
4. **Hostile-input edges.** A few library tests per parser (ACPI, PCI
   configuration, NVMe, FAT32, ELF64, path walk, paging tables, VT-d) that pin
   a real edge or a past bug. The tests deleted from a parser stay in git
   history as seeds for fuzzing.
5. **Fixed-bug regressions.** A test that exists because a real bug happened
   stays. None is pruned.
6. **Tooling smoke.** About ten host tests: the image builds and boots, serial
   verdict parsing on one good and one bad transcript, the reproducible
   evidence round trip, the command line refusing a bad option, and any save or
   data round trip a user can hit.

Everything else is deleted: a test that restates the code, asserts a constant,
a table or a layout, checks wiring, getters or types, snapshots a whole output,
checks that a mock was called or duplicates a kept check. A flaky or slow check
is deleted, not fixed. A change adds at most one focused test, and only for a
real bug.

Source rules are lints, `deny` or `forbid` attributes or types, never tests that
read source text. The `unsafe` rules are `unsafe_code` (`forbid` in every
`libs/` crate, `deny` in `kernel`), `undocumented_unsafe_blocks`,
`multiple_unsafe_ops_per_block` and `missing_safety_doc`; `kernel` and `wost`
also refuse `debug_assert` through `disallowed_macros`, so no production
invariant is debug-only.

`cargo mutants` over changed library code or a library before a release, and
fuzzing of the tier 4 parsers after v0.3.0, are laptop work run on demand. They
never block a commit, and nothing requires or records them.
The mutation registry, `cargo xtask mutate`, the cargo-mutants grade, the
owned deep security scan, the API caller audit, the qualification records, the
source contract tests, the prose and decision pin tests and the hashed `unsafe`
table are removed.

The release check is: `cargo xtask ci` green, `cargo xtask evidence` and
`cargo xtask verify-evidence` on the exact commit, a VMware boot of the
self-test and shipped images read through `cargo xtask qualify-vmware`, and a
person reading the release diff. `cargo xtask model-check` runs when a model
changes. D-425's release gate is replaced by this check. D-441's source-preview
class stays, without its decision and documentation test checks, which no longer
exist, and a release no longer carries qualification records. Evidence binds the
commit and its reproduced artifacts only, so the `parent-docs-only-v1` relation
of D-451's rule 5 and the qualification bundle it related to end. The decision
reader and the documentation pin tests that D-451's Consequences describe are
removed; decision numbering and the index are kept by review.

**Consequences.** `AGENTS.md`, `CONTRIBUTING.md` and `005-code-conventions.md`
state the rules above. D-453 item 5 already ends the mutation per new check and
keeps one witness, a boot smoke check and a focused test per real bug between
milestones; this decision ends the registry, the sweep and the other machinery
that item left standing. Test counts
and mutation ids leave the documents. The bodies of the superseded decisions
stay unedited as history; they describe machinery that no longer exists. The
evidence format and the CI stage list change, so a bundle written before this
decision does not verify against newer source. The `kernel/src/selftest.rs`
count constants, the library test suites and the self-test checks are cut in
the units that follow.

**Alternatives.** Keeping the registry and cutting only the tests leaves the
largest cost, a ledger every change must move. Keeping the deep scan and the
caller audit as non-blocking tasks keeps a multi-day owned run and a
compiler-evidence parser that only the audit's own tests exercise. A suite
without the hostile-input tier would delete the tests that found real bugs.
