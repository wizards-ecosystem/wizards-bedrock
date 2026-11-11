# D-441 Experimental source previews have a separate publication contract

Status: Accepted, 2026-10-25. Amends D-26 and supersedes only D-425's first-v0.1.0 assignment.

**Decision.** There are two release classes. An **experimental source preview**
publishes reviewed source and contribution infrastructure with a bounded
source/build check record and disclosed limitations. It establishes no phase
closure, hardware qualification, stable ABI, completed security audit or
production readiness. The first v0.1.0 is this class: an immutable annotated
tag, GitHub prerelease, not Latest, with normal tagged source downloads and no
new kernel image, VM appliance or qualification bundle. Existing reviewed
vendored build inputs and their licenses stay in the source tree.

A **qualified diagnostic release** retains D-425's complete local CI, complete
curated mutation sweep, model checking, fresh advisory review, both fresh
VMware selectors, reproducibility, and creation plus independent verification
of exact-source evidence. The independent grade and owned Deep campaigns
retain their exact-source pass/deferred rules and sole existing PASS authority.
No gate, parser, record format, mutation witness, hardware admission or source
binding changes. This ADR supersedes only D-425's assignment of that stronger
contract to the first v0.1.0. Its history and qualification contract remain.
A later qualified release needs another immutable version; v0.1.0 never moves.

For this launch's documentation, community and source-preview packaging changes
only, the full-CI-per-commit rule is replaced by disclosure/license review,
patch whitespace checks, `cargo xtask fmt --check`, `cargo xtask supply-chain`,
`cargo xtask verify-vendor`, `cargo xtask build --release`, and the existing
focused decision/documentation/registry tests on one clean candidate commit.
Retain commands, SHA, tools, outcomes and limitations outside tracked source.
The automated build/test budget is 20 minutes in aggregate. If a required
check fails or cannot finish, safe source visibility may proceed after
disclosure clearance, but the release stays unpublished. No VM boot is
required and a compile supplies no boot claim. This exception does not cover
substantive kernel changes or future release qualification.

D-26 is amended only to permit the read-only **Source preview checks** hosted
workflow: pinned toolchain, one Windows GNU runner, existing bounded source
checks, no secrets or publishing, and ordinary fork pull requests. It is not
the local CI gate or release qualification. Observe success before considering
a required status; preserve existing protection and maintainer access.

Disclosure review covers the tracked candidate, all exposed branch/tag history
and material repository-owned surfaces before public visibility. A confirmed
secret, unauthorized private material, unresolved redistribution right or
unreviewed material exposure blocks visibility; removing it from HEAD is not
history remediation. No history rewriting is authorized by this policy.

**Consequences.** All existing foundation rows keep their actual states and
proof obligations. Realm retains product/compiler-readiness ownership, Ink
retains WZ blockers, and stopped triggers remain stopped. Source-preview
results are plain reports, never v11 evidence or qualification-record PASS.
The supply-chain policy separates source previews from qualified binary
distribution, including its fresh advisory and accompanying notices duties.

**Rationale.** Public source and outside contributions need a truthful entry
point before the open kernel foundation and hardware qualification finish, and
D-425 assigned the full qualified contract to the first v0.1.0 publication,
which postponed collaboration behind hours of machine qualification. Silently
waiving that contract would confuse publication with qualification, a public
mirror would lose the existing history, and a movable preview tag would obscure
which source was checked.
