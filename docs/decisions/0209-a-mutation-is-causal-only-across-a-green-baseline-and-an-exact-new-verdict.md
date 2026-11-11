# D-209 A mutation is causal only across a green baseline and an exact new verdict

Status: Accepted, 2026-08-28. Amends D-119, D-134 and D-175; closes PW-504.

**Decision.** Before writing any remaining mutation, each invocation runs one
green baseline for every witness class those mutations need: the complete host
suite, the release self-test image and the degraded release image. A resumed
invocation first restores its active sandbox file and then reruns the relevant
baselines. Baseline success is intentionally not journalled; after a killed
process, repeating the evidence is what proves the restored build cache and
source still produce green artifacts. If any baseline is red, panics, times
out, fails to build or misses a required marker, the sweep stops before
journalling or writing a mutation.

The self-test harness returns a typed observation to the mutation runner:
`Passed`, `Failed` with the exact ordered check names, or `Panicked` with the
exact serial panic transcript. Ordinary `test-boot` still accepts only `Passed`.
Against the green baseline, a `Check` mutation is caught only by a final failed
verdict containing its complete check name, and a `Panic` mutation only by a
panic transcript containing its registry-pinned marker. A passed mutated run is a
survivor. A different failed check, different panic, malformed verdict, build
failure, timeout or harness failure is inconclusive and never credited. The
degraded-image witnesses retain their exact missing-marker comparison, now
after one baseline which proved every required marker was present.

The baseline is per witness class, not per mutation. Every mutation in an
invocation starts from the same byte-exact snapshot and is restored before its
outcome is durable, so repeating an identical clean boot hundreds of times
adds no causal information. One baseline per invocation adds at most the host
suite and two boots, while a resume deliberately pays that bounded cost again.

**Rationale.** D-134 observed only the mutated run, so a check already red in
the pristine source was credited, an unrelated failure that stopped the named
check was called a survivor, and any panic was credited to a panic mutation.
D-208 makes a pristine, commit-pinned source snapshot available before every
fresh or resumed invocation, so the missing causal comparison is finite.
Causality needs both sides of the transition. “This check is red
after the edit” does not establish that the edit made it red; “the pristine
artifact passed, then this exact check or panic marker appeared” does. Treating
a wrong failure as inconclusive also keeps two different repairs distinct: a
survivor needs a stronger check, while a wrong verdict needs a narrower or
corrected mutation. Typed observations keep that distinction out of formatted
`anyhow` text.

**Alternatives.** Trusting the last `ci` run was rejected because it may name a
different commit, target directory or process lifetime and is not resumable
evidence. A baseline before every mutation was rejected because the immutable
snapshot makes those runs identical and the extra machine hours prove nothing
new. Recording only a generic panic was rejected because a mutation can panic
earlier for an unrelated reason. Requiring the named check to be the only red
check was rejected because several deliberate defects legitimately violate
multiple downstream invariants; the causal claim is about the named transition,
not the mutation's entire blast radius.

**Consequences.** `cargo xtask mutate` now restores any active mutation and
then proves the full host suite, release self-test image and/or degraded release
image green exactly once for the witness classes still ahead. The self-test
harness preserves a typed pass, all named failures, or the complete serial
panic transcript; it rejects a failed count whose names are incomplete. The
runner credits only the registered check name or panic marker and reports every
other failure as inconclusive rather than caught or survived.

Release QEMU measured the three terminal causes as
`faulting address 0x0000600000000000`,
`a trap handler may not take a thread lock`, and
`address 0xffffffffffffffe8`; each is pinned in the registry. Six persistent host
mutations keep a red baseline, a wrong check, a wrong panic, an erased panic
continuation, unnamed failures and an omitted exact panic expectation red.
