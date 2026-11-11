# D-309 Semantic parser witnesses precede later shape validation

Status: Accepted, 2026-09-12. Amends D-244, D-297 and D-301; advances PW-001.

**Decision.** Give each semantic rejection a witness whose input satisfies
every later structural contract. Duplicate-key insertion is tested directly on
`Record` before manifest rendering. The unrelated-error and runtime-failure
witnesses place one hostile line in an otherwise valid clean TLC transcript;
clean verdicts have no behavior trace whose adjacency the added line could
disturb. Each retained mutation names its dedicated test.

**Rationale.** Three survivors weakened semantic rejections whose old fixtures
were still refused later, by canonical rendering or by exact trace-shape
validation, so each broad error assertion proved only that some later contract
remained.

**Alternatives.** Removing the integrated hostile-input cases was rejected
because they still exercise the complete parser path. Relaxing canonical
rendering or trace shape for a test was rejected because a witness must isolate
one contract without weakening another production defense.
