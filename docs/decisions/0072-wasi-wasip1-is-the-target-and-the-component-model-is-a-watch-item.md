# D-72 WASI: `wasip1` is the target, and the Component Model is a watch item

Status: Provisional, 2026-08-23. Amended by D-181. Amends D-09.

**Context.** D-09 made WASI 0.3 components a first-class app format with
**wasmi** as the runtime. WASI 0.3.0 shipped on 2026-06-11; the rest of that
sentence has problems.

The Component Model is still a **Phase 1 proposal** in the W3C WebAssembly CG,
the lowest active phase of five. "Component Model 1.0" is a roadmap article
whose open workstreams include moving the ABI off eager allocation and
"simplifying the spec… to reduce implementation burden": the Bytecode Alliance
calls its specification too hard to implement, and its ABI will change. Only
Wasmtime and jco implement WASI 0.3, and D-09 already rejects Wasmtime as
unusable in-kernel.

wasmi has no Component Model or plans for one. Its maintainer, September 2025:
*"an absolutely huge proposal and therefore an enormous undertaking for any
Wasm runtime to implement… it remains uncertain if a Wasm interpreter will
ever even profit."* Its tracking issue, open since January 2024, left the
planned list in February 2025.

Measured: the Component Model spec is ~12,200 lines on top of core Wasm, and
Wasmtime's implementation is ~40,900 lines of Rust, larger than wasmi's entire
core interpreter. The published ecosystem is three sample HTTP servers and
seven micro-utilities.

**Decision.** Core Wasm plus **`wasip1`** on a wasmi-class interpreter, with
WASI host bindings written directly over native handles. The Component Model
becomes a watch item with a named trigger: **it reaches Phase 4, or an
interpreter ships support.**

**Rationale.** `wasip1` delivers both of D-09's reasons: WASI's authority
model maps onto handle grants, and a component built for this OS survives an
architecture port. D-181 narrows the first claim: a WASI descriptor is an
adapter object over a handle, with descriptor numbering, preopens and cursor
state that the cursorless native Read does not carry. The authority model
aligns; the descriptor contract is not impedance-free. The Component Model
adds cross-language typed *composition*, an ecosystem feature, for an
ecosystem of ten components.

`008` §4.5 asked to soften the "wasmi 1.0" pin, never actioned; this
supersedes it. Runtime selection happens when Phase 7 starts.

**Consequences.**

- Phase 7's WASI work shrinks from "implement the Component Model" (costed at
  zero in the original plan) to "wasip1 host bindings over handles".
- `002-architecture.md`'s app-formats section is restated to match.
- Provisional, per D-71: revisit on the trigger above.
