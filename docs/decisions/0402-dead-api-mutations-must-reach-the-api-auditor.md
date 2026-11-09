# D-402 Dead-API mutations must reach the API auditor

Status: Accepted (2026-10-13), amends D-213 and D-395; advances PW-001

**Context.** Permanent mutation 1,821 restored the retired
`SelectedUsesCatchAll` requester-inventory error variant and expected the API
audit to name that exact variant as having no caller. Adding the variant made
the crate's exhaustive `Display` match fail to compile, so the audit never
constructed its public-item inventory and the mutation was inconclusive. The
compiler refusal is safe, but it does not exercise the claimed API-audit
witness. Production contains neither the retired variant nor another public
way to select the obsolete catch-all refusal.

**Decision.** Keep production unchanged and recalibrate mutation 1,821 to add
a compilable public `selected_uses_catch_all` helper that returns the current
`IncludesAllPci` refusal. The helper restores the same obsolete public policy
choice without changing enum exhaustiveness. Its only acceptable witness is
the API audit's structured verdict naming exactly that one dead function.

**Proof required.** The restored source must pass the complete API audit.
Applying mutation 1,821 alone must compile every audited graph and fail with
exactly one dead item, `fn selected_uses_catch_all`; restoring the source must
return the audit to green. Registry application checks, the ordinary gate and
the next complete clean sweep must retain that result.

**Alternatives.** Treating any compile error as an API-audit catch was rejected
because it would let an unrelated syntax or type failure satisfy a caller
proof. Adding a wildcard arm to production `Display` was rejected because it
would weaken exhaustive handling of security refusal reasons. Adding both the
variant and a formatting arm was rejected because the formatting arm itself
would be a caller and could hide the dead public surface the mutation is meant
to expose.
