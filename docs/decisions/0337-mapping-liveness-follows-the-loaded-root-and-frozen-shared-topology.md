# D-337 Mapping liveness follows the loaded root and frozen shared topology

Status: Accepted, 2026-09-28. Amends D-125, D-142, D-253, D-259, D-281 and D-284; closes PW-120 and advances PW-540.

**Context.** The current implementation had repaired two related paging trust
boundaries without a current end-to-end causal record. Stack and device tokens
must describe what the loaded page tables will reach, not merely rewalk an old
root which still shares kernel entries. Separately, every user root must inherit
one reserved device-window topology before activation, and later binding may
ignore only the `ACCESSED` bit which hardware can set. Local evidence was still
needed before closing the former; the latter also needs a fresh VMware
observation of the real shared-root path.

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

**Evidence.** On 2026-09-28 retained mutations 817, 818 and 821 through 825
were applied individually and restored byte-exact. Replacing the current-root
lookup with recorded-root checks failed only `a stale shared root cannot
authenticate a device mapping` and `a stale shared root cannot authenticate a
stack`, respectively, each at 261/262. Skipping the stack's middle or final
leaf failed only its corresponding named check; skipping the device mapping's
first, middle or final leaf likewise failed only its corresponding named check,
each at 261/262. D-259's 2026-08-31 mutation 819 record independently skipped
the first stack leaf and failed all three then-new retained-stack substitution
checks before restoration passed 251/251.

For PW-540, mutations 326, 327 and 1,311 through 1,313 were then applied one at
a time and restored byte-exact. Omitting pre-activation reservation prevented
the I/O APIC and both NVMe controllers from mapping, failed 43 dependent
self-checks including `late device mappings reach earlier user roots`, then
reached the complete terminal panic `the bound-space witness did not retire`.
Allowing a new active top-level table failed only `active kernel topology
refuses a new shared root` at 261/262. Accepting a huge present root, accepting
a user present root and constructing a user table each failed only `shared
topology rejects hostile root entries` at 261/262.

Mutation 1,314 compared the complete raw table word and failed the exact host
identity test because accessed-only drift was rejected. Mutation 1,315 ignored
`NO_CACHE` as well and the same test failed with `bit 4 was omitted from table
identity`. Exact restoration passed that one test with 54 filtered out,
release and debug QEMU each passed 262/262, and the applicability and named-check
tests passed for all 1,412 registered mutations.

**Alternatives.** Trusting a token's saved root was rejected because the next
hardware access follows the loaded root. Sampling only an endpoint was rejected
because an interior leaf is independently substitutable. Creating the shared
slot lazily was rejected because active user roots already copied the older
topology. Comparing raw entry words was rejected because hardware owns
`ACCESSED`; ignoring another cache or privilege bit was rejected because it
would turn materially different translations into the same identity.
