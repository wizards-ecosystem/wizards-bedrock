# D-429 A complete independent grade matches its exit to adjudicated outcomes

Status: Accepted, 2026-10-23. Qualification OPEN; amends D-373; advances the PW-025 continuous campaign.

**Decision.** After the complete candidate/outcome bijection, passing pristine
baseline, phase replay and exact tracked adjudications have been verified,
the owned campaign requires exit 3 when the parsed timeout count is nonzero,
otherwise 2 when the parsed missed count is nonzero, otherwise 0. Unviable
mutants alone do not change that required zero. The boundary also requires
caught plus adjudicated to equal the complete parsed total. Any mismatch,
signal or other exit remains a refusal. This amends D-373's blanket nonzero
refusal; it does not admit incomplete runs, unreviewed survivors or a failed
baseline. Final source and executable remeasurement still follows inventory
and doctests and immediately precedes the sole owned PASS door.

A focused witness exercises the complete counter/exit truth table, including
timeout precedence over missed mutants and unviable-only success, and rejects
usage, baseline and internal errors. Another refuses missing or surplus
adjudications despite an otherwise matching exit. The ordering witness requires
parsing, exact adjudications and the exit boundary before final identity
verification. Retained mutation 1,748 continues to remove the exit predicate;
mutations 2,322 through 2,326 name the adjudication total, exit precedence,
missed classification, unviable classification and owned-call boundary. Each
must be observed red before it is credited; on 2026-10-23 each was, on Windows,
where the separate Unix signal witness was not executed.

**Consequences.** The tracked adjudication policy is still empty because no
observed independent campaign is available to adjudicate. D-425 continues to
permit an explicitly deferred grade in a release bundle; this implementation
does not claim a passing grade.

**Rationale.** The pinned cargo-mutants 27.1.0 grader exits 2 for missed
mutants and 3 for timeouts, with timeout taking precedence, including outcomes
the tracked policy permits as equivalent or bounded nontermination, so
rejecting every nonzero exit before reading outcomes made those dispositions
unreachable. Accepting 0, 2 or 3 without checking the counters would accept a
contradictory result, suppressing all nonzero exits would accept tool
failures, and treating an unobserved survivor as equivalent would replace the
required review with an assumption.
