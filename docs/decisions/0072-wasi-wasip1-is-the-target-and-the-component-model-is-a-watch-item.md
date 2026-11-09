# D-72 WASI: `wasip1` is the target, and the Component Model is a watch item

Status: Provisional (2026-08-23) (amended by D-181), amends D-09

**Context.** D-09 made WASI 0.3 components a first-class app format with
**wasmi** as the runtime. WASI 0.3.0 did ship, on 2026-06-11. Everything else
in that sentence has a problem.

The Component Model is still a **Phase 1 proposal** in the W3C WebAssembly CG,
the lowest active phase of five. "Component Model 1.0" is a roadmap article
whose open workstreams include transitioning the ABI away from eager
allocation, and "simplifying the spec… to reduce implementation burden" — the
Bytecode Alliance saying its own specification is too hard to implement and its
ABI is going to change. WASI 0.3 is implemented by Wasmtime and jco only, and
D-09 already rejects Wasmtime as unusable in-kernel.

And wasmi has no Component Model and no plans for one. Its maintainer, in
September 2025: *"an absolutely huge proposal and therefore an enormous
undertaking for any Wasm runtime to implement… it remains uncertain if a Wasm
interpreter will ever even profit."* Its tracking issue has been open since
January 2024 and was removed from the planned list in February 2025.

The scale, measured: the Component Model spec is ~12,200 lines on top of core
Wasm, and Wasmtime's implementation of it is ~40,900 lines of Rust — larger
than wasmi's entire core interpreter. The published component ecosystem is
three sample HTTP servers and seven micro-utilities.

**Decision.** Core Wasm plus **`wasip1`** on a wasmi-class interpreter, with the
WASI host bindings written directly over native handles. The Component Model
becomes a watch item with a named trigger: **it reaches Phase 4, or an
interpreter ships support.**

**Rationale.** Both reasons D-09 exists survive and are delivered by `wasip1`:
WASI's authority model maps onto handle grants, and a component compiled for
this OS survives an architecture port. D-181 narrows the first claim: a WASI
descriptor is an adapter object over a handle, with descriptor numbering,
preopens and cursor state that the cursorless native Read does not carry. The
authority model aligns; the descriptor contract is not impedance-free. What
the Component Model adds is cross-language typed *composition*, an ecosystem
feature, and the ecosystem is ten components.

`008` §4.5 asked for the "wasmi 1.0" pin to be softened and it was never
actioned; this supersedes that. Runtime selection happens when Phase 7 starts.

**Consequences.**

- Phase 7's WASI work shrinks from "implement the Component Model" — costed at
  zero in the original plan — to "wasip1 host bindings over handles".
- `002-architecture.md`'s app-formats section is restated to match.
- Provisional, per D-71: revisit on the trigger above.
