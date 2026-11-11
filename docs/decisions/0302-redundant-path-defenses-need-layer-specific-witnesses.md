# D-302 Redundant path defenses need layer-specific witnesses

Status: Accepted, 2026-09-12. Amends D-263 and D-301; advances PW-001.

**Decision.** Retain both defenses and give each an observer that reaches only
its layer. The pre-open fixture installs a redirect first and uses the existing
post-measurement hook to prove the file was never opened. The post-read fixture
opens a direct file, replaces its pathname with a symlink through that hook,
proves the substitution completed and requires the final identity check to
refuse it. Each mutation names only its layer-specific test:
`supply_chain_text_rejects_a_redirect_before_opening` and
`supply_chain_text_revalidates_its_path_after_reading`.

**Rationale.** Two survivors shared one SPDX symlink fixture that installed the
redirect before the read began, so either check could reject it. The checks
are deliberately redundant against that static input but own different
moments: the first prevents following a redirect at open, the second detects
pathname substitution after the checked handle exists.
