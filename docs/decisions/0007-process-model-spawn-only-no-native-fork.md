# D-07 Process model: spawn-only, no native fork

Status: Accepted, 2026-07-18. Amended by D-111.

**Decision.** Native process creation is spawn-only; D-111 narrows its birth
authority to one bootstrap channel endpoint. Bootstrap grants arrive through
it, and later authority may also come from a handle-authorized ring
operation (D-151, D-163). No native fork. A gated address-space clone
remains Phase 9 compatibility work, not a primitive the current kernel
provides.

**Rationale.** Native fork would add an implicit wholesale inheritance path
beside the explicit grants this kernel audits, plus a second way to
reproduce thread and address-space state ("A fork() in the road", HotOS'19).

**Consequences.** mlibc's fork-dependent ports get the slower emulated path;
native software uses spawn.
