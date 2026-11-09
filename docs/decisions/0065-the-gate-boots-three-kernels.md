# D-65 The gate boots three kernels

Status: Accepted, 2026-08-23. A fourth added by D-90.

**Context.** The 2026-08-22 audit's other standing lesson was that a gate is
only as wide as its configuration matrix. This one found the matrix two
configurations short. `ci` booted the self-test kernel and *built* the shipped
one; and it was release-only, where arithmetic wraps in silence.

The first gap had grown teeth. The self-test build exits at the debug-exit
device the moment it has a verdict, so nothing it does covers what the shipped
kernel does afterwards — and since D-61 made the boot thread a blocking
keyboard reader, that is everything the kernel does at all.

**Decision.** Three boots. The self-test kernel in release, the same in debug,
and the shipped image under `boot-check`, which requires it to reach its
steady state and then survive five more seconds without panicking. Fifteen
seconds of extra wall clock between them.

**Rationale.** Both gaps were found by reading rather than by failing, which
is the argument for closing them: a hole in the gate is invisible from inside
it. The debug boot is the only configuration in which an overflow on the
machine goes red rather than quietly wrong, and it caught one on its first
run — in the round counter written that hour, where `then_some` evaluates its
argument even at zero.

**Consequences.**

- A debug boot gets 150 seconds where release gets 60. A debug kernel is
  several times slower under emulation — a console scroll costs the better
  part of a second — and holding it to the release budget reports load as a
  hang, which it did once.
- `boot-check` has no debug-exit device to report through, so its verdict is
  what the serial log says: a settled marker, and no panic line. Two markers
  kept in step with the kernel by comment, like `READY_MARKER` before them.
- Both new steps were broken before being believed: a panic before settling, a
  panic after settling, and a kernel that never says it settled.
