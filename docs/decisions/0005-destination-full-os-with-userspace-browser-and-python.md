# D-05 Destination: full OS with userspace, browser, and Python

Status: Accepted, 2026-07-18.

**Context.** Sets the architecture bar and multi-year scope.

**Decision.** The endpoint is a complete OS: own userland and shell,
networking, GUI with own compositor, and two flagship port ladders —
Python (RustPython early → CPython later) and web browsing (ported
`links` text browser → NetSurf graphical). Pace: massive multi-year
project; milestones favor always-finishable increments.

**Rationale.** CPython and NetSurf are the classic proof-of-real-OS
trophies and force every subsystem (FS, network, graphics, libc) to
become real. RustPython runs pre-libc (pure Rust), giving scripting
powers early; links proves the network stack before graphics exist.

**Consequences.** Demands the POSIX translation layer (D-10) no matter
how novel the native ABI is.

**Amended by D-75.** NetSurf remains the integration target, but "browser" and
"web browsing" are historical shorthand. The current milestone is document-web
rendering and proof that libc, windowing, networking, and TLS compose.

**Amended by D-180.** Phase 7 extends the emitted native caller into a compiled
Rust delegation probe, then provisionally tests Wizzy as the first
language/runtime for native system programs. RustPython moves to the Phase 9
compatibility-runtime work; the Python destination and CPython acceptance
target do not change.

**Amended by D-183.** Wizzy is the settled default for feasible first-party
target-side userland. The compiler-readiness gate precedes product work; the
Rust probe remains an oracle rather than an alternate `init`, shell or tool set.
