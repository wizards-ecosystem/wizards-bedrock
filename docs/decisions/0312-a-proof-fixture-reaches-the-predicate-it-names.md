# D-312 A proof fixture reaches the predicate it names

Status: Accepted (2026-09-12), amends D-244 and D-301; advances PW-001

**Context.** Five complete-sweep survivors remained in host checks whose broad
fixtures stopped before, or passed around, the changed predicate. The NVMe
source-order witness bounded the whole preparation path and therefore accepted
a register write inside final construction before the cleanup owner existed.
Qualified workspace type matching had a second path predicate which implied the
one mutated. Three copies of the kernel-entry grammar could disagree, while the
alternate fixture renamed only the macro argument and was rejected earlier for
having no matching handler. Unknown item-macro shadowing and active dependency
build-script rejection were each credited through integrated callers with
other refusal doors.

**Decision.** Bound the NVMe order proof to `finish_preparation` and require no
register write before its `Controller` cleanup owner. Remove the redundant
qualified-type clause. Parse the exact kernel-entry grammar once, retain the
parsed handler in `EntrySpec`, and resolve that retained name; the alternate
fixture changes both declaration and invocation so only the `kernel_main`
identity guard can refuse it. Exercise item-macro shadow policy and active
build-script validation directly before their integrated callers.

Keeping broader fixtures as the only witnesses was rejected because their
verdicts do not identify which contract refused the input. Retaining duplicate
type and entry predicates was rejected because no mutation of one redundant
layer can prove it is load-bearing.

**Proof.** Injecting the early NVMe write failed
`nvme_cleanup_and_bus_master_authority_precede_the_enable_write`. Discarding
workspace-qualified identity failed
`workspace_qualified_trait_implementations_bind_to_the_declaring_crate`.
Admitting a different entry handler failed
`exact_kernel_entry_reaches_its_handler_without_trusting_other_macro_tokens`.
Trusting an unknown item macro and omitting active build scripts respectively
failed
`an_item_macro_disables_unqualified_transparency_across_its_caller_target` and
`unreviewed_dependency_build_scripts_are_outside_the_audit_authority` at their
new direct assertions. Each exact mutation was restored and its focused test
passed; registry applicability and named-check validation passed. The successor
sweep remains PW-001.
