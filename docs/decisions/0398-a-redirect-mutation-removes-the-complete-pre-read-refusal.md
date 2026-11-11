# D-398 A redirect mutation removes the complete pre-read refusal

Status: Accepted, 2026-10-13. Amends D-302, D-322 and D-344; advances PW-001.

**Decision.** Keep both production defenses. Mutation 905 removes the
complete pre-read redirect refusal for the supply-chain reader: it removes the
`require_direct_file` check and replaces `open_real_file` with an ordinary
`File::open`. The unmodified reader refuses the static redirect before the
fixture's hook runs; under the mutation the fixture must observe that its
post-measurement hook ran, proving the redirected target was opened before the
final post-read path check refused it, so
`supply_chain_text_rejects_a_redirect_before_opening` fails. Mutation 906
continues to remove only that final revalidation and keeps the
substitution-after-open fixture as its independent witness.

**Rationale.** D-344 routed the reader through `open_real_file`, which
independently refuses a final-component link, so the D-302 mutation that
removed only the direct-file check became equivalent and its survival did not
show that production accepts redirected text. The path check gives an early
diagnostic while the opened-file primitive binds and validates the actual
handle across its check-to-open race, so neither is deleted. A third
parser-level rejection would miss that the property is filesystem authority,
not text syntax.
