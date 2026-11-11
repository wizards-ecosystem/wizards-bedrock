# D-279 User-copy recovery has one fixed test driver, not a general unchecked copy

Status: Accepted, 2026-09-04. Amends D-120 and D-252; closes PW-538.

**Decision.** Production copies continue to walk every page and require its
effective `USER` permission before the assembly window opens. The general
address-and-slice unwalked API is removed. A self-test-only, zero-argument
driver instead verifies one module-private address is unmapped, copies from
that fixed hole into module-private sentinel scratch, and reports only whether
the complete remainder was returned with the scratch unchanged. Neither the
address, bytes nor partial result crosses its API.

D-252's exact opaque recovery target and D-126's separate self-test fault-window
observer remain unchanged. The former confines the trap-frame rewrite; the
latter proves a ring-3 instruction in that interval retires rather than taking
the ring-0 recovery path. Recoverability now justifies the walked copy's
map-to-use race, not arbitrary access to the lower half.

**Rationale.** D-120 concluded that recovering a fault made any lower-half
unwalked read safe. That range check proves neither user permission nor
confidentiality: a safe address space can contain a supervisor-only leaf below
`USER_END`, and ring 0 can read it. The general door had no production caller;
only the recovery machine check used it.

**Alternatives and evidence.** Making the old door unsafe would retain
an unnecessary caller obligation, while driving the test through the walked
copy would refuse before reaching the recovery mechanism. Shipped compiler
probes reject both the removed door and the test driver. Substituting a
completed-copy remainder, corrupting the private scratch and admitting a
supervisor-only leaf each fail only their named machine check, and the
fixed-hole trap-arm mutation remains the terminal-panic witness from D-120.
Concurrent revocation remains the future PW-702 trigger.
