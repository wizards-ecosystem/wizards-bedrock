# D-07 Process model: spawn-only, no native fork

Status: Accepted, 2026-07-18. Amended by D-111.

**Context.** Follows from D-06. `fork()` implicitly duplicates the whole
authority and address-space state, and its snapshot semantics compose badly
with multiple threads.

**Options.** Classic fork/exec; spawn-only with compat-fork emulation.

**Decision.** Native process creation is spawn-only; D-111 narrows its birth
authority to one bootstrap channel endpoint. Bootstrap grants arrive through
it, and later authority may also come from a handle-authorized ring
operation (D-151, D-163). No native fork. A gated address-space clone remains
Phase 9 compatibility work, not a primitive the current kernel provides.

**Rationale.** "A fork() in the road" (HotOS'19) set out the case, and
Fuchsia shipped a spawn-oriented process model. Native fork would add an
implicit wholesale inheritance path beside the explicit grants this kernel
audits, plus a second way to reproduce thread and address-space state.
Capsicum supports namespace confinement and handle-like process descriptors,
but it is not spawn-only evidence: capability mode is inherited across both
`fork` and `pdfork`.

**Consequences.** mlibc's fork-dependent ports get the slower emulated path;
native software uses spawn and is better for it.
