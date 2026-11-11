# D-310 Mutation witnesses target one load-bearing predicate

Status: Accepted, 2026-09-12. Amends D-244 and D-301; advances PW-001.

**Decision.** Represent every non-call `use` with one classifier. Remove the
duplicate re-export ranges, collector state, query and observation branch; the
retained mutation now disables the sole general guard. Its fixture keeps both a
public re-export and a function-local unused import, so neither can silently
become caller evidence.

Extract the host-library coverage predicate and exercise it both across the
real registry and with a one-entry synthetic registry whose source package and
witness package differ. The retained coverage mutation removes only the
package-identity comparison and names the synthetic test.

**Rationale.** Two survivors removed no asserted invariant: `NonCallUses`
already recorded every `ItemUse`, so a second re-export guard was redundant,
and reassigning one EEVDF mutation left the existential per-package coverage
check true while other EEVDF entries qualified.

**Alternatives.** Keeping two identical `use` classifiers was rejected because
it adds state and makes either guard non-load-bearing. Naming one arbitrary
EEVDF registry entry as permanent coverage was rejected because an existential
property is not violated while another qualifying entry exists.
