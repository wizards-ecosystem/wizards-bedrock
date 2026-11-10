# D-09 App formats: ELF64 + WASI 0.3 components as first-class

Status: Provisional, 2026-07-18. Provisional since 2026-08-23: restated by D-72 as wasip1-first.

**Context.** "Ahead of its time" directive; a sandboxing story was needed.

**Options.** ELF only; design-for-WASM-later; WASI first-class now.

**Decision.** Two first-class formats: native ELF64 and WASI Component
Model apps. Target **WASI 0.3** (ratified 2026): its async model is
completion-based and host-driven, like io_uring/IOCP, and maps nearly 1:1
onto our native queues. First runtime: **wasmi 1.0** (API-stable
Dec 2025, no_std, register-machine interpreter, differentially fuzzed
against wasmtime). We write the WASI host bindings directly over native handles.
wasmtime's host implementation is tokio/std-coupled and unusable in-kernel;
its cap-std/WasiCtx pattern is the shape to follow.

**Rationale.** The authority a WASI descriptor carries is a capability
grant, which aligns with the handle model. D-181 later records why the
descriptor is not literally a native handle: numbering, preopens and cursor
state belong to an adapter. Sandboxed, portable apps still avoid porting a
whole toolchain.

**Consequences.** The syscall/handle layer must let the WASI host be a pure
userspace/kernel-service consumer of it (no special kernel hooks). wasmi
has no built-in WASI, so the host is our code.
