# D-333 Final user entry is tested through the production xstate predicate

Status: Accepted, 2026-09-28. Amends D-270; advances PW-132.

**Decision.** The final-door witness suppresses
`xstate::user_state_is_isolated()` itself while executing the production
preflight, and its synthetic test-only branch is removed. A retained mutation
replacing only the final call with remembered `is_enabled()` fails only
`missing extended state closes the user boundary`, binding that call
independently of D-270's early-door mutation. The reviewed `task.rs` unsafe
inventory stays 14 blocks and three declarations. PW-132 stays open for its
remaining row-level causal proofs.

**Rationale.** D-270 requires early construction and the final ring-3 crossing
to ask the same live extended-state predicate, but the final-door witness
withheld a separate test-only discriminator, so replacing its production call
with `is_enabled()` left that witness green.
