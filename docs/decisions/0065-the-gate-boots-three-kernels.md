# D-65 The gate boots three kernels

Status: Accepted, 2026-08-23. A fourth added by D-90.

**Context.** The 2026-08-22 audit's other standing lesson: a gate is only as
wide as its configuration matrix. This audit found it two configurations
short. `ci` booted the self-test kernel and *built* the shipped one, and
only in release, where arithmetic wraps silently.

The first gap had grown teeth. The self-test build exits at the debug-exit
device once it has a verdict, so it never covers what the shipped kernel
does afterwards. Since D-61 made the boot thread a blocking keyboard reader,
that is everything the kernel does.

**Decision.** Three boots: the self-test kernel in release, the same in
debug, and the shipped image under `boot-check`, which must reach its
steady state and survive five more seconds without panicking. They add
fifteen seconds of wall clock.

**Rationale.** Both gaps were found by reading, not failing, which argues
for closing them: a hole in the gate is invisible from inside. The debug
boot is the only configuration where an overflow on the machine goes red
instead of quietly wrong. It caught one on its first run, in the round
counter written that hour, where `then_some` evaluates its argument even at
zero.

**Consequences.**

- A debug boot gets 150 seconds where release gets 60. A debug kernel is
  several times slower under emulation (a console scroll costs most of a
  second), and the release budget would report load as a hang, which it did
  once.
- `boot-check` has no debug-exit device to report through, so its verdict
  is what the serial log says: a settled line, and no panic line. Comments
  keep both in step with the kernel, like `READY_MARKER` before them.
- Both new steps were broken before being believed: a panic before
  settling, a panic after settling, and a kernel that never says it settled.
