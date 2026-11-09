# D-311 Boundary witnesses fail at the named guard

Status: Accepted (2026-09-12), amends D-244 and D-301; advances PW-001

**Context.** Complete-sweep mutations 651, 659, 770 and 937 survived because
their witnesses could pass without exercising the changed predicate. The API
target fixture supplied a missing repository, so source canonicalization failed
before target preparation. Model scratch validation repeated a parent equality
already guaranteed by the preceding real-directory ancestry check. The PCI
fixture combined BAR and command failures but its command case exercised leased
command restoration rather than the final command write of an unleased BAR
probe. The API collector nested an implementation in a module, which the file
visitor reached without recursing through an enclosing implementation.

**Decision.** Expose narrow target and scratch-boundary helpers to their local
tests so each redirected path is refused before later setup or destructive
work. Retain the shared real-directory ancestry check as the sole model scratch
boundary and remove its redundant parent comparison. Give the final unleased
probe command restoration a dedicated ignored-write and retry-authority test.
Place the nested inherent implementation inside an outer implementation's
method so finding it requires the exact visitor recursion under test.

Keeping broad end-to-end error assertions was rejected because an earlier or
later guard could continue satisfying them. Retaining the redundant model
parent predicate was rejected because no filesystem state can pass the shared
ancestry validation and then violate it. Reusing the leased command fixture was
rejected because the two restoration paths hold different authority.

**Proof.** Replacing API target preparation with unchecked directory creation
made the direct redirected-target assertion accept the link. Replacing model
scratch ancestry validation with bare canonicalization made its direct
assertion accept the linked external directory before recursive deletion could
run. Accepting any final probe command readback made
`an_ignored_final_probe_command_restore_retains_retry_authority` receive a
successful probe instead of retained restoration authority. Disabling visitor
recursion made `nested_inherent_methods_remain_visible_to_api_contracts` lose
the nested method. All four exact tests pass after restoration; the registry
source and check-name proofs pass. The remaining complete-report outcomes and
a clean successor sweep stay in PW-001.
