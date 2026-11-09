# D-302 Redundant path defenses need layer-specific witnesses

Status: Accepted (2026-09-12), amends D-263 and D-301; advances PW-001

**Context.** Complete-sweep mutations 886 and 887 each survived the same SPDX
symlink fixture. That fixture installed the redirect before the read began, so
either the pre-open direct-file check or the post-read revalidation could
reject it. The two checks are deliberately redundant against that static
input, but they own different moments: the first prevents following a redirect
at open, while the second detects pathname substitution after the checked
handle exists. One test could not prove either layer independently.

**Decision.** Retain both defenses and give each an observer that reaches only
its layer. The pre-open fixture installs a redirect first and uses the existing
post-measurement hook to prove the file was never opened. The post-read fixture
opens a direct file, replaces its pathname with a symlink through that hook,
proves the substitution completed and requires the final identity check to
refuse it. Each mutation names only its layer-specific test.

**Proof.** Removing the pre-open check made
`supply_chain_text_rejects_a_redirect_before_opening` fail because its hook ran.
After restoration, removing the final check made
`supply_chain_text_revalidates_its_path_after_reading` fail because the read
returned successfully after the fixture substituted the pathname. Both exact
tests pass with the defenses restored, and the registry source and check-name
proofs pass. The remaining complete-report outcomes and a clean successor
sweep stay in PW-001.
