# D-316 Designation is authorization, and requirements and grants are two documents

Status: Accepted, 2026-09-12. Refines D-111 and D-183; resolves the placement fork in `013-application-platform.md`.

**Context.** D-111 gives a program exactly one handle at birth. `013` reserved
an ELF note namespace for what a program requires and left open whether a
program's requirements and a deployment's grants are one document or two,
because no package format existed. `015` fixed that effects are requests and
never grants. Three independent traditions in `018` section 3.4 (the powerbox,
Access Control Gadgets, Sculpt's component graph) reached the same rule for
the user interface: derive authority from what the user is already doing and
show the graph, never ask. Fuchsia's manifest-only routing, which asks nobody,
is documented to encourage over-permissioning; macOS's re-consent prompts are
documented to loop.

**Decision.** Requirements are derived by the compiler, carried in the D-226
envelope as `effects.required`, copied into the package manifest, and never
hand-edited past the image build. Grants are the user's decision, kept by the
bench in a per-program grant document that names the artifact digest it was
made against. The ELF `PT_NOTE` reservation stays empty. A first launch, or a
version whose requirements changed, shows the difference against the grant
document and records the answer; a non-interactive session refuses instead.
At run time there are no prompts: a missing holding is a typed refusal that
the toolkit turns into the gesture that would grant it. The gestures that
grant are the bench-drawn picker, a drop from Files, a plumbing message
carrying an attenuated holding, and, as an experiment, a drop of one window
onto another. The compositor draws a trust label in every window's title bar
and owns that region. The Ledger app lists holdings, grants and refusals per
program; its revoke ends the program until the kernel has epoch revocation,
which `019` section 15 names as a prerequisite with the Ledger as caller. A
component that acts without a user in the loop is marked `agent`, runs in
restricted mode, is logged under its own identity and cannot grant to itself.

**Rationale.** Two documents because they are two things: a claim a program
makes about itself and a decision its user made. Deltas because lists get
rubber-stamped and deltas get read. No prompts because a prompt asks the
user about authority instead of deriving it from designation, and the
evidence that this fails is thirty years old.

**Alternatives.** Requirements in the executable alone were rejected because a
deployment's policy must be able to differ from a build. Runtime prompts were
rejected on the evidence above. A single merged document was rejected because
a program could then edit its own grant.

**Consequences.** `013`'s decision 5 is resolved. The bench needs a picker, a
drop protocol and a label region before any app; the plumber attenuates.
Kernel epoch revocation is a named post-gate prerequisite.
