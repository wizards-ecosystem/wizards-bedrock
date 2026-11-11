# D-376 Dynamic mapping admission rejects W+X before page-table preflight

Status: Accepted, 2026-10-10. Amends D-12, D-44 and D-105; closes PW-585.

**Decision.** `MapError::WriteExecute` names the requested virtual base.
`map_range` constructs the complete present flag set and rejects it when
`Flags::is_write_execute` is true, before protected-stack inspection,
page-table preflight, table-frame allocation or leaf writes. Every
caller-controlled dynamic door (`map_boot`, `map_owned`, `map_user`, `map_shared` and
`map_shared_ring`) converges on this private function; `set`, the only
production leaf writer, is private beneath it. Invalid ranges still fail their
architecture checks first. Read-only executable mappings and writable
`NO_EXECUTE` mappings retain their existing encodings and ownership rules.

A live-machine check submits the hostile flag shape through all five safe
dynamic doors, requires the dedicated refusal at each address, observes every
target absent, proves caller-owned frames return, proves only the fresh root
table existed and conserves complete allocator ownership. Its adjacent control
maps writable `NO_EXECUTE` pages through the owned, user, shared and boot paths.
Mutation 1,766 removes `WRITABLE` before the policy test; on 2026-10-10 it
failed only `writable executable mappings leave no trace`.

**Rationale.** Codex Security scan `32245094-b8b7-47d1-b6e7-a52554800772` found
that the safe dynamic mapping doors accepted caller-selected `WRITABLE` without
`NO_EXECUTE`, so a safe caller could create a present writable/executable page.
The image audit at initial activation is not an admission boundary for later
owned, user or shared mappings. Silently adding `NO_EXECUTE` would reinterpret
the request instead of refusing unsafe state; a check at every public door could
drift from future callers and private aliases; and the raw `Entry` encoder would
mix an admission rule into the representation used to inspect and construct
exact hardware state. The shared private mapping boundary is the narrowest
complete enforcement point.
