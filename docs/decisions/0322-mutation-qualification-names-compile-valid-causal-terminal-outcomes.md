# D-322 Mutation qualification names compile-valid, causal terminal outcomes

Status: Accepted, 2026-09-12. Closes the `f28428a` repair inventory subject to a clean sweep.

**Decision.** Every registered replacement must compile in the graph its
witness runs. A machine mutation is caught only by its registered exact check,
an exact causal panic marker, or expiry of its exact registered self-test
deadline. A pristine green self-test remains a prerequisite. A timeout is
credited only when the top-level error is exactly the deadline error; a build
failure, emulator failure, different duration, panic or named red is
inconclusive. Terminal registrations say why an ordinary verdict is
unreachable.

Ordinary self-test boots continue to require all interactive markers exactly
once and in order. Mutation observation may advance past a missing earlier
marker so it can harvest a later exact red verdict, but rejects duplicate or
backward markers and skips the missing stage's stale keystrokes. Pure defects
use their exact host tests. A named failure printed before later corruption or
panic remains causal evidence; fixtures publish that failure before dependent
destructive probes.

**Rationale.** The 1,359-entry sweep on `f28428a` left 24 inconclusive
outcomes after D-302 through D-313 repaired its survivors, and they were not
one problem: some replacements did not compile, some reached a causal panic,
deadlock or missing interactive stage before a verdict, and some named the
wrong check or a machine check where a host oracle was the direct witness.
A compiler refusal says nothing about the intended property,
and an unrelated red, panic or delay can make a broken registry look strong.
Conversely, refusing to recognize a deterministic deadlock or abandoning the
serial stream at the first missing interaction hides the actual consequence
of a load-bearing mutation. Exact terminal classes retain the same standard
as exact check names without requiring a doomed kernel to continue.

**Alternatives.** Treating every non-pass as caught was rejected because it
credits build failures and unrelated crashes. Requiring every defect to reach
the final verdict was rejected because fail-stop ownership violations and
deadlocks are themselves the causal outcome. Weakening ordinary CI's marker
order was rejected because only the mutation observer needs recovery.

**Consequences.** Three harness mutations keep timeout identity,
forward-only recovery of a missing stage and stale-round suppression
load-bearing. PW-001 remains open until one clean current
sweep reports every entry caught with no survivor or inconclusive result.
