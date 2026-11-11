# D-09 App formats: ELF64 + WASI 0.3 components as first-class

Status: Provisional, 2026-07-18. Provisional since 2026-08-23: restated by D-72 as wasip1-first.

**Decision.** Two first-class formats: native ELF64 and WASI Component Model
apps. Target **WASI 0.3**, whose completion-based async model maps nearly
1:1 onto our native queues. First runtime: **wasmi 1.0** (no_std,
differentially fuzzed against wasmtime). We write the WASI host bindings
directly over native handles, following wasmtime's cap-std/WasiCtx pattern;
its tokio/std-coupled host is unusable in-kernel.

**Rationale.** The authority a WASI descriptor carries is a capability
grant, which aligns with the handle model; D-181 records why the descriptor
is not literally a native handle (numbering, preopens and cursor state
belong to an adapter).

**Consequences.** The syscall/handle layer must let the WASI host be a pure
userspace/kernel-service consumer of it (no special kernel hooks).
