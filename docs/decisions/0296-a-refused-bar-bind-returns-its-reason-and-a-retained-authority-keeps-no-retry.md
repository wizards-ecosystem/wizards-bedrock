# D-296 A refused BAR bind returns its reason, and a retained authority keeps no retry accessor

Status: Accepted (2026-09-06), amends D-249 and D-285; closes PW-546

**Context.** The first complete audit after D-291 and D-295, with every
dispatch resolved, reported three public items with no caller:
`BarAuthorityFailure::reason` and `BarAuthorityFailure::into_authority` in
`pci-config`, and `BarBindFailure::into_range` in `wost`. D-285 moved the
kernel to retaining a refused BAR-authority release whole, in its bounded
quarantine, so nothing reads the reason or recovers the authority for a retry
that no pre-Wizzy path makes. A refused bind carried the admitted device
range back to the kernel, which discarded it: `AdmittedDeviceRange` is a
proof this CPU derives from the immutable boot map and its own width, holds
no ledger entry and has no `Drop`, so keeping it buys nothing and dropping it
loses nothing.

**Decision.** `wost::arch::x86_64::pci::bind_memory_bar` returns
`Result<AdmittedBar, BarBindError>`; the `BarBindFailure` wrapper is gone and
the kernel maps the error directly. `BarAuthorityFailure` keeps its private
reason and authority, its `Display` and `Error` implementations, and loses
both accessors. The quarantine still holds the authority, so a future reclaim
path adds its accessor together with its caller, which is the only order the
audit admits.

**Alternatives.** Manufacturing a caller, such as logging the reason at the
quarantine, was rejected because a log line is not a design need and would
hide the next dead accessor behind the same trick. Keeping the range in the
bind refusal under a retained-owner name was rejected because a re-derivable
proof is not an owner, and calling it one would misstate what a refusal can
lose.

**Proof.** The audit that found the three items is the check; its two
retained mutations already prove it can fail. Clippy on every registered
graph, the seventy-two `pci-config` tests and the reviewed-unsafe inventory
pass after the removal, and the complete gate on the unchanged tree closes
PW-546.
