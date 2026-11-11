# D-210 Every pure crate keeps one load-bearing break alive

Status: Accepted, 2026-08-28. Amends D-28 and D-189; closes PW-503.

**Decision.** Every current `libs/` package carries at least one persistent
mutation of production source, caught by one exact test in that same package.
The selected mutation is the highest-consequence stable transformation found
in each previously unrepresented crate:

| Crate | Deliberate defect retained | Exact witness |
|---|---|---|
| `eevdf` | an overrun slice wraps into an enormous remainder | `a_thread_past_its_deadline_has_no_slice_left` |
| `elf64` | a segment extent wraps into the image | `a_loader_can_ask_whether_the_contents_fit` |
| `frame-space` | a multi-frame free mutates before the whole run is valid | `freeing_a_run_is_all_or_nothing` |
| `mmio-range` | one byte past a region is admitted | `a_span_one_byte_past_the_region_is_refused` |
| `path-walk` | `..` at the root is clamped rather than refused | `ascending_past_the_start_is_refused_not_clamped` |
| `pixel` | a padded framebuffer is mapped by visible-row bytes | `a_padded_framebuffer_spans_its_pitch` |
| `ps2-keys` | set 1 swallows the left-shift release as a reply | `set_1_reads_aa_as_a_shift_release_not_a_self_test_result` |
| `ring-proto` | admission ignores occupied completion slots | `the_reservation_invariant_holds_under_interleaving` |
| `slab-layout` | an over-aligned zero-size allocation asks for zero frames | `a_zero_sized_request_too_aligned_for_a_class_still_gets_a_frame` |
| `stack-layout` | a stack begins where its guard begins | `every_stack_has_its_guard_immediately_below` |
| `timebase` | updates discard their fractional nanosecond carry | `a_thousand_updates_do_not_drift_from_one_conversion` |
| `trap-cause` | the write-fault reader tests the user bit | `every_page_fault_bit_is_read_on_its_own` |
| `x86-cpuid` | a compacted-state offset truncates through `u32` | `a_layout_that_overflows_a_u32_offset_is_not_consistent` |
| `x86-paging` | permission intersection becomes union | `intersection_keeps_only_what_both_hold` |
| `x86-pit` | calibration floors instead of rounding to nearest | `rounding_is_to_nearest_not_floor` |
| `x86-tables` | a user-callable IDT gate receives the wrong privilege bits | `a_user_callable_gate_differs_only_in_privilege` |

A host repository check derives the package domain from `HOST_PACKAGES` and
requires every non-`xtask` package to appear in a `HostTest` mutation. Its own
mutation removes one package from that coverage. This is a floor, not a quota:
existing crates with several independent high-risk transactions keep several
mutations, and a new property still gets its own mutation under D-119.

Only transformations that apply exactly once, compile, execute the intended
test and make that test's own verdict red are admitted. Mutating a test,
depending on timing, breaking compilation, or relying on another test's
failure proves nothing and remains inconclusive under D-209.

**Rationale.** Host and `libs/` checks were broken once when written, but
D-119 originally admitted only machine mutations, and D-187 later admitted exact
host tests for the device-transaction work, leaving the older break evidence
historical. Sixteen of the twenty-three pure crates, including the scheduler,
loader, allocators and CPU table encodings, had no live break, so a change
could make their checks stop reaching production code unnoticed. D-209 supplies
the pristine host baseline and exact-test attribution. One live defect per pure
crate proves the complete path from
production source through that crate's tests and the mutation runner. It does
not claim every assertion is independently mutation-tested; that would turn
PW-503 into an unbounded score. Choosing authority, bounds, ownership and
hardware-encoding defects keeps the floor about consequences rather than line
coverage.

**Alternatives.** Keeping the historical red records was rejected because
they cannot detect a check that stops reaching its subject. Registering every
old hand break was rejected because many are duplicate routes to one property,
unstable textual edits, or test-fixture mutations. Selecting only the newest
storage crates was rejected because those seven are already represented while
the scheduler, memory and ABI foundations are the uncovered part.

**Consequences.** Sixteen production mutations give every pure crate at least
one live break, a seventeenth removes `eevdf` from the derived coverage floor,
and an eighteenth admits a successful command that selected no test. The
witness name is measured, not assumed: the ring defect left
`admission_stops_before_a_completion_could_be_dropped` green and was caught only
by `the_reservation_invariant_holds_under_interleaving`. Host attribution
requires the exact test's own `ok` or `FAILED` line, so a successful zero-test
run is inconclusive and earns no mutation credit.
