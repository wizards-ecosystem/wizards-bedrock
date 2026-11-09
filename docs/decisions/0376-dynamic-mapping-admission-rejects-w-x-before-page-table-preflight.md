# D-376 Dynamic mapping admission rejects W+X before page-table preflight

Status: Accepted, 2026-10-10. Amends D-12, D-44 and D-105; closes PW-585.

**Context.** A Codex Security finding from scan
`32245094-b8b7-47d1-b6e7-a52554800772` found that the safe dynamic mapping
doors accepted caller-selected `WRITABLE` without `NO_EXECUTE`.
`AddressSpace::map_range` added `PRESENT`, preflighted the request and wrote a
leaf without re-establishing D-12's W^X invariant. The image audit at initial
activation could detect a bad boot-time kernel mapping, but it was not an
admission boundary for later owned, user or shared mappings. A safe caller
could therefore create a present writable/executable page.

**Decision.** `MapError::WriteExecute` names the requested virtual base.
`map_range` constructs the complete present flag set and rejects it when
`Flags::is_write_execute` is true, before protected-stack inspection,
page-table preflight, table-frame allocation or leaf writes. Every
caller-controlled dynamic door (`map_boot`, `map_owned`, `map_user`, `map_shared` and
`map_shared_ring`) converges on this private function; `set`, the only
production leaf writer, is private beneath it. Invalid ranges still fail their
architecture checks first. Read-only executable mappings and writable
`NO_EXECUTE` mappings retain their existing encodings and ownership rules.

**Proof required.** A live-machine check must submit the hostile flag shape
through all five safe dynamic doors, require the dedicated refusal at each
address, observe every target absent, prove caller-owned frames return, prove
only the fresh root table existed and conserve complete allocator ownership.
Its adjacent ordinary control must map writable `NO_EXECUTE` pages through
the owned, user, shared and boot paths. One retained mutation must remove the
write bit before the central predicate and fail the hostile check.

**Evidence.** Before the guard, the new release-machine witness alone failed
while all 279 older checks passed; it observed the W+X boot mapping and its
new tables. After the guard, the focused release machine passed 280/280.
Mutation 1,766 removed `WRITABLE` before the policy test and failed only
`writable executable mappings leave no trace` at 279/280; exact restoration
returned to the green source. The mutation-registry identity test passed.
Final integrated-gate evidence is recorded by the closing commit.

**Alternatives.** Silently adding `NO_EXECUTE` would reinterpret the caller's
request instead of refusing unsafe state. Repeating the rule at every public
door would leave future callers and private aliases able to drift. Enforcing
the policy only during activation would miss live additions. Putting the
policy in the raw `Entry` encoder would mix a kernel admission rule into the
low-level representation used to inspect and construct exact hardware state.
The shared private mapping boundary is the narrowest complete enforcement
point.
