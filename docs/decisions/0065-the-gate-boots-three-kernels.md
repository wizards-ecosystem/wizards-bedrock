# D-65 The gate boots three kernels

Status: Accepted, 2026-08-23. A fourth added by D-90.

**Decision.** Three boots: the self-test kernel in release, the same in
debug, and the shipped image under `boot-check`, which must reach its steady
state and survive five more seconds without panicking.

**Rationale.** A gate is only as wide as its configuration matrix (D-22).
The self-test build exits once it has a verdict, so it never covers what the
shipped kernel does afterwards, which since D-61 is everything. The debug
boot is the only configuration where an overflow on the machine goes red
instead of quietly wrong. A hole in the gate is invisible from inside.

**Consequences.**

- A debug boot gets 150 seconds where release gets 60. A debug kernel is
  several times slower under emulation, and the release budget would report
  load as a hang.
- `boot-check` has no debug-exit device to report through, so its verdict is
  what the serial log says: a settled line, and no panic line. Comments keep
  both in step with the kernel, like `READY_MARKER` before them.
