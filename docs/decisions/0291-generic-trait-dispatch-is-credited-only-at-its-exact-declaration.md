# D-291 Generic trait dispatch is credited only at its exact declaration

Status: Accepted (2026-09-06), amends D-275; advances PW-544

**Context.** The first current full API audit refused valid generic calls such
as `<T as Trait>::member` because rustc can identify the trait declaration but
cannot name one implementation while type-checking the generic body. The same
run exposed the opposite error in the external-trait fallback: an untyped
method call could activate an unrelated local implementation merely because
its member had the same name. Treating every generic diagnostic as trusted
would have hidden the ambiguous receiver cases D-275 intentionally refuses.

**Decision.** Every dispatch records compiler evidence separately for the
callee and the syntactic `Self` locations. A type-qualified static call prefers
the exact non-alias nominal type in its callee over argument zero and carries
that one resolved type into implementation matching; the evidence set cannot
cross-product two independently mentioned types.

One declaration-only route exists for explicit qualified calls whose `T` is a
lexical type parameter with that direct trait bound, either on the parameter or
in its `where` clause. The direct path keeps its complete module qualifier while
stripping only the terminal trait's generic arguments. Compiler evidence must
name exactly that audited trait-member declaration and no concrete self type or
implementation. The route credits the declaration but activates no
implementation body. Indirect supertraits, higher-ranked bounds, aliases,
associated or parameterized `Self` types, and every other ambiguous shape
retain D-275's exact-one rule and fail closed when it cannot be met.

For an explicit qself path call, argument zero is an ordinary argument and
never receiver-type evidence. A full trait path is never reduced to its
terminal name. An exact local type may credit the one selected local default
only when no applicable or unresolved override exists. A compiler-selected
inherent method or free function suppresses unrelated trait routing. An
external trait implementation requires direct implementation evidence or an
exact `Self` together with an explicit trait qualifier; bare method syntax and
a matching member name are insufficient. One `DispatchResolution` value drives
both liveness and refusal reporting so the two consumers cannot disagree.

**Alternatives and proof obligation.** Per-site allowlists and replacing the
type-safe generic APIs with concrete wrappers were rejected because they move
an audit limitation into production design. Activating every candidate was
rejected because it would turn uncertainty into false reachability. Focused
fixtures cover direct and `where` bounds, parameterized direct bounds, the
static-associated-function argument trap, concrete evidence at a generic site,
rejected indirect and non-parameter shapes, a local default beside a sibling
override, inherent/free-function collisions, explicit and untyped external
dispatch, and the earlier ambiguity/specialization refusals. They also drive
the shared resolution through both liveness and failure reporting.

Fifteen independent breaks each made its exact fixture red before restoration:
qself argument zero, declaration-only routing, name-only activation, concrete
Self admission, parameterized and full qualifiers, direct-bound membership,
callee/Self evidence partition, local defaults, ordinary-call exclusion,
external qualifier admission, both shared-resolution consumers, callee
priority and same-terminal trait paths. Those defects remain mutations 1,328
through 1,342. Earlier dispatch and constructor-span mutations whose anchors
moved were re-anchored and re-proven rather than dropped. The complete 123-test
debug API family passes. The release family, real API audit and full current CI
remain required; no result from the failed run is promoted to passing evidence.
