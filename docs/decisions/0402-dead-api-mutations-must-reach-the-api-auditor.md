# D-402 Dead-API mutations must reach the API auditor

Status: Accepted, 2026-10-13. Amends D-213 and D-395; advances PW-001.

**Decision.** Keep production unchanged and recalibrate mutation 1,821 to add
a compilable public `selected_uses_catch_all` helper that returns the current
`IncludesAllPci` refusal. The helper restores the same obsolete public policy
choice without changing enum exhaustiveness. Its only acceptable witness is
the API audit's structured verdict naming exactly that one dead function: the
mutation must compile every audited graph and fail with exactly one dead item,
`fn selected_uses_catch_all`.

**Rationale.** Restoring the retired `SelectedUsesCatchAll` variant made the
exhaustive `Display` match fail to compile, so the audit never built its
inventory and the mutation was inconclusive. Treating any compile error as an
API-audit catch would let an unrelated failure satisfy a caller proof, a
wildcard `Display` arm would weaken exhaustive handling of security refusal
reasons, and a formatting arm for the variant would itself be a caller and hide
the dead surface the mutation exposes.
