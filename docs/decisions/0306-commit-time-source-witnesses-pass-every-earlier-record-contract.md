# D-306 Commit-time source witnesses pass every earlier record contract

Status: Accepted, 2026-09-12. Amends D-244 and D-301; advances PW-001.

**Decision.** The fixture authors a complete current model record, including
every registered transcript artifact, while retaining the real commit and tree
captured from its private Git repository. After changing a tracked file, it
requires the specific stable-clean-checkout error from the immediate pre-rename
source capture. Production source binding and the retained mutation are
unchanged.

**Rationale.** The old handcrafted record lacked transcript artifacts the
schema had since gained, so `validate_record` refused it before the final
source recheck ran, and a broad error assertion credited that earlier failure.

**Alternatives.** Weakening record validation for this test was rejected
because a commit-time witness must first satisfy every contract that precedes
commit. Checking only that no final record exists was rejected because any
early return could satisfy that consequence without exercising source identity.
