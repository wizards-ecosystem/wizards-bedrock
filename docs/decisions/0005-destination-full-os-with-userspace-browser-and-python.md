# D-05 Destination: full OS with userspace, browser, and Python

Status: Accepted, 2026-07-18.

**Decision.** The endpoint is a complete OS: own userland and shell,
networking, GUI with own compositor, and two flagship port ladders: Python
(RustPython early, CPython later) and web browsing (ported `links` text
browser, then NetSurf graphical). Pace: massive and multi-year; milestones
favor always-finishable increments.

**Rationale.** CPython and NetSurf force every subsystem (FS, network,
graphics, libc) to work, which demands the POSIX translation layer (D-10)
however novel the native ABI is.

**Amended by D-75.** NetSurf remains the integration target; "browser" and
"web browsing" are historical shorthand. The current milestone is
document-web rendering and proof that libc, windowing, networking and TLS
compose.

**Amended by D-180.** Phase 7 extends the emitted native caller into a
compiled Rust delegation probe, then provisionally tests Wizzy as the first
language/runtime for native system programs. RustPython moves to Phase 9
compatibility-runtime work; the Python destination and CPython acceptance
target do not change.

**Amended by D-183.** Wizzy is the settled default for feasible first-party
target-side userland. The compiler-readiness gate precedes product work; the
Rust probe remains an oracle, not an alternate `init`, shell or tool set.
