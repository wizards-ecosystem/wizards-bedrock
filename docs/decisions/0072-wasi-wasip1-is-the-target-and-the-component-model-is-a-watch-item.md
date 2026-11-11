# D-72 WASI: `wasip1` is the target, and the Component Model is a watch item

Status: Provisional, 2026-08-23. Amended by D-181. Amends D-09.

**Decision.** Core Wasm plus **`wasip1`** on a wasmi-class interpreter, with
WASI host bindings written directly over native handles. The Component Model
becomes a watch item with a named trigger: **it reaches Phase 4, or an
interpreter ships support.** This supersedes `008` §4.5's unactioned request
to soften the "wasmi 1.0" pin; runtime selection happens when Phase 7
starts.

**Rationale.** The Component Model is a Phase 1 proposal whose ABI will
change, only Wasmtime and jco implement WASI 0.3, and wasmi has no plans for
it. `wasip1` delivers both of D-09's reasons: WASI's authority model maps
onto handle grants, and a component built for this OS survives an
architecture port. D-181 narrows the first claim: a WASI descriptor is an
adapter object over a handle, with descriptor numbering, preopens and cursor
state that the cursorless native Read does not carry. The authority model
aligns; the descriptor contract is not impedance-free.

**Consequences.**

- Phase 7's WASI work is "wasip1 host bindings over handles", not "implement
  the Component Model".
- `002-architecture.md`'s app-formats section is restated to match.
- Provisional, per D-71: revisit on the trigger above.
