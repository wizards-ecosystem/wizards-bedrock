# D-310 Mutation witnesses target one load-bearing predicate

Status: Accepted, 2026-09-12. Amends D-244 and D-301; advances PW-001.

**Context.** Complete-sweep mutations 426 and 446 survived because neither
removed the invariant its named test asserted. Mutation 426 disabled a public
re-export guard, but `NonCallUses` independently records every `ItemUse`,
including public re-exports, and the immediately following general guard still
discarded the same diagnostic. Mutation 446 reassigned one EEVDF mutation to a
foreign test package, but EEVDF had other correctly assigned source mutations,
so the existential per-package coverage check remained true.

**Decision.** Represent every non-call `use` with one classifier. Remove the
duplicate re-export ranges, collector state, query and observation branch; the
retained mutation now disables the sole general guard. Its fixture keeps both a
public re-export and a function-local unused import, so neither can silently
become caller evidence.

Extract the host-library coverage predicate and exercise it both across the
real registry and with a one-entry synthetic registry whose source package and
witness package differ. The retained coverage mutation removes only the
package-identity comparison and names the synthetic test.

Keeping two identical `use` classifiers was rejected because it adds state and
makes either guard non-load-bearing. Naming one arbitrary EEVDF registry entry
as permanent coverage was rejected because an existential property is not
violated while another qualifying entry exists.

**Proof.** Disabling the unified non-call-`use` guard made
`reexports_docs_and_same_names_are_resolved_by_the_compiler` reject the first
re-export diagnostic for lacking an executable caller. Removing witness-package
identity made
`a_foreign_host_witness_cannot_cover_a_librarys_source_mutation` accept the
foreign synthetic entry. Both exact tests pass after restoring their sole
predicates; the registry source and check-name proofs pass. The remaining
complete-report outcomes and a clean successor sweep stay in PW-001.
