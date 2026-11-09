# D-398 A redirect mutation removes the complete pre-read refusal

Status: Accepted, 2026-10-13. Amends D-302, D-322 and D-344; advances PW-001.

**Context.** Permanent mutation 905 removed the supply-chain reader's
pre-open `require_direct_file` check and survived its static redirected-file
fixture. D-344 subsequently routed the same reader through `open_real_file`,
which independently refuses a final-component link before any caller-visible
read. The original D-302 mutation therefore became redundant: removing only
one of two pre-open defenses cannot make the redirected input observable, and
the survivor does not show that production accepts redirected supply-chain
text.

**Decision.** Keep both production defenses. Mutation 905 now removes the
complete pre-read redirect refusal for this reader: it removes the direct-file
check and replaces `open_real_file` with an ordinary `File::open`. The existing
layer-specific fixture must then observe that its post-measurement hook ran,
proving the redirected target was opened before the final post-read path check
refused it. Mutation 906 continues to remove only that final revalidation and
keeps the substitution-after-open fixture as its independent witness.

**Proof required.** The unmodified reader must refuse the static redirect
before its hook runs. Applying mutation 905 alone must make
`supply_chain_text_rejects_a_redirect_before_opening` fail because the hook ran,
then restoring the source must return the exact test to green. The registry
application check and the next complete clean sweep must retain that result.

**Alternatives.** Deleting either production defense was rejected because the
path check gives an early diagnostic while the opened-file primitive binds and
validates the actual handle across its check-to-open race. Adding a third
parser-level rejection was rejected because
the security property is filesystem authority, not text syntax. Crediting the
survivor as a product vulnerability was rejected because the opened-file
primitive still refused the exact redirect before its bytes reached a parser.
