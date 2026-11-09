# D-295 A type-qualified external-trait call is resolved by its callee type's one audited implementation

Status: Accepted (2026-09-06), amends D-291; advances PW-544

**Context.** The first optimized full audit refused thirteen live sites of
the form `Type::from(value)`: `wost::mm::DmaSegment::from`,
`mm::USegment::from` and `EnableFailure::from` in the kernel, its self-tests
and `pci-config`. Each names an exact audited type in its callee path and
calls a `core::convert::From` implementation that the audited crate declares
exactly once for that type. D-291 admitted an external trait implementation
only with an explicit `<Type as Trait>::member` qualifier, because bare method
syntax can auto-reference into a different implementation and a matching
member name alone is not evidence. A static path call has neither weakness:
the compiler resolves `Type::member` against that one type's inherent items
and then the traits implemented for it, and an inherent or free selection is
already visible to the audit and already excludes trait routing.

**Decision.** At a static path call whose callee path names an exact,
non-alias, non-generic audited type and carries no explicit `qself`, an
external-trait activation for that same type is matched exactly as the
explicitly qualified form is; the type's audited implementation inventory
supplies the qualifier. D-275's exact-one rule then decides: one matching
implementation is credited, and two implementations of one generic trait for
one type still fail closed until the call site is written in the qualified
form. Method syntax, explicit `qself` without a trait, aliased callees and
generic callee arguments keep D-291's refusal, and argument zero never
supplies the `Self` of a path call whose callee could not, so an aliased
callee fails closed instead of borrowing its argument's type.

**Why this is exact.** Two different external traits offering one member
name for one type make `Type::member` a compile error, so they cannot reach
the audit. The remaining competitor is a `core` blanket implementation of the
same trait for every type. The reflexive `From` and `TryFrom` conversions
cannot survive the gate because clippy's `useless_conversion` is denied, and
the `Into`, `TryInto`, `ToOwned`, `ToString` and `Borrow` blankets can only
mis-credit a site whose audited crate implements that trait directly beside a
second conversion to the same type, which no audited crate does. The
qualified form of D-291 does not compare trait generic arguments either, so
this route is neither wider nor narrower than it.

**Alternatives.** Rewriting the thirteen sites as `<Type as From<_>>::from`
was rejected because it changes production source to fit an audit
limitation, the reason D-291 itself refused per-site allowlists. Crediting
every candidate was rejected because it turns an ambiguity into false
reachability. Crediting by member name without the callee's type evidence was
rejected because it is the D-291 defect in another form.

**Proof.** Three fixtures cover the admitted call, a two-implementation
refusal that counts both candidates, and an aliased callee that refuses
rather than borrowing argument zero. Two manual breaks each reddened only
their own fixture before restoration and remain mutations 1,352 and 1,353;
the two D-291 mutations whose anchor this decision moved were re-anchored and
re-proven. The real audit and the complete gate on the unchanged tree remain
PW-544's closing evidence.
