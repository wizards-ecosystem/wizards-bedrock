# D-337 Mapping liveness follows the loaded root and frozen shared topology

Status: Accepted, 2026-09-28. Amends D-125, D-142, D-253, D-259, D-281 and D-284; closes PW-120 and advances PW-540.

**Decision.** A retained stack or device mapping is live only when its recorded
root resolves through the current mapping root and every page in its complete
extent still has the original physical address, four-kilobyte span and exact
required flags. The first, interior and final leaves are all identity-bearing.

Activation reserves the device-window PML4 slot as a supervisor table before
publishing a root. Once active, the kernel cannot create another higher-half
top-level table. An existing shared slot is accepted only when it is present,
non-huge and supervisor-only; a new one is constructed supervisor-only. Binding
requires the same table address and every entry flag except `ACCESSED`.

D-125's remaining live-root and complete-stack audit disposition is closed, and
PW-120 leaves D-142 and the hostile-surface audit. PW-540 remains `READY` until
the same restored implementation passes VMware; local causal closure does not
substitute for that hardware observation.

**Rationale.** Stack and device tokens must describe what the loaded page
tables will reach, not rewalk an old root that still shares kernel entries,
and every user root must inherit one reserved device-window topology before
activation, with later binding ignoring only the `ACCESSED` bit hardware can
set.

**Evidence.** Retained mutations 817, 818 and 821 through 825 substitute the
recorded root or skip one stack or device leaf, and each fails only its
stale-root or leaf check; D-259's mutation 819 skips the first stack leaf. For
PW-540, mutations 326, 327 and 1,311 through 1,313 omit the pre-activation
reservation, allow a new active top-level table or accept or construct a huge
or user root, and each fails its named topology check or terminal panic.
Mutations 1,314 and 1,315 compare the raw table word or also ignore
`NO_CACHE`, and each fails the exact host identity test.

**Alternatives.** Trusting a token's saved root was rejected because the next
hardware access follows the loaded root. Sampling only an endpoint was rejected
because an interior leaf is independently substitutable. Creating the shared
slot lazily was rejected because active user roots already copied the older
topology. Comparing raw entry words was rejected because hardware owns
`ACCESSED`; ignoring another cache or privilege bit was rejected because it
would turn materially different translations into the same identity.
