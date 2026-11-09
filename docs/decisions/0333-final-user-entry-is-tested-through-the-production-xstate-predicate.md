# D-333 Final user entry is tested through the production xstate predicate

Status: Accepted, 2026-09-28. Amends D-270; advances PW-132.

**Context.** D-270 requires early construction and the final ring-3 crossing
to ask the same live extended-state predicate, but the final-door witness
withheld a separate test-only discriminator. Replacing its production call
with remembered `is_enabled()` therefore left that witness green, and the
retained substitution mutation covered only the early door.

**Decision and evidence.** The final-door witness now suppresses
`xstate::user_state_is_isolated()` itself while executing the production
preflight. Its obsolete synthetic branch is removed. Replacing only the final
call with `is_enabled()` then failed only `missing extended state closes the
user boundary` at 261/262; exact restoration passed 262/262. The new retained
mutation binds that final call independently of D-270's early-door mutation,
and the reviewed `task.rs` unsafe inventory remains 14 blocks and three
declarations. PW-132 stays open for its remaining row-level causal proofs.
