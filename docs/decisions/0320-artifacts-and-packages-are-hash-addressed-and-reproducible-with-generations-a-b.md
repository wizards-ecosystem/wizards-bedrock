# D-320 Artifacts and packages are hash-addressed and reproducible, with generations, A/B images and a safe mode

Status: Accepted, 2026-09-12. Refines D-73 and D-226.

**Decision.** A program ships as a reproducible artifact in the D-226
envelope; a package is an immutable, content-addressed set of components with
a signed provenance record and the derived manifests of D-319; the store is
addressed by digest. Until WizFS the image is read-only and the store ships
inside it. Installation, when it arrives, is profile mutation: a profile is a
document naming a user's components and grant documents, every change is a new
generation, and generations of the profile and of the base image are the two
axes of one Updates panel showing current, previous and rollback. The image is
A/B with a verified inactive slot and a fallback boot; safe mode boots the
current image with every user layer disabled.

**Rationale.** `015` requires two clean builds to agree byte for byte and keeps
package installation behind WizFS (D-73), and `018` found generations, A/B
images with a fallback boot and a safe mode to be settled practice. The D-226
envelope already carries the digest, so addressing the store by it costs
nothing and makes reproducibility a property of the package format rather than
a later audit. Deciding the shape now keeps the image build honest about what
it consumes.

**Alternatives.** A mutable package database was rejected because rollback
then means backup. Application bundles without a store were rejected because
they cannot deduplicate or verify. Deferring the decision to Phase 13 was
rejected because the image build consumes artifacts in Phase 7.

**Consequences.** The image build's digest check is the first half of this
decision and exists; the profile and the Updates panel wait for WizFS. `019`
section 11 has the details.
