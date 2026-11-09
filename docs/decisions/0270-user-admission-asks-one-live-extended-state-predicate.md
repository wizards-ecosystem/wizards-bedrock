# D-270 User admission asks one live extended-state predicate

Status: Accepted, 2026-08-31. Amends D-82, D-115, D-228 and D-245; advances PW-132.

**Context.** A remembered nonzero save form did not prove that live CR4 and
XCR0 still matched the area which had been sized. Boot could also enable a
strict subset of the fixed x87/SSE/AVX policy, and a fresh task restored zero
bytes under a contract which named only save-produced images.

**Decision.** Initialization accepts only the complete exact fixed component
mask. Standard-form size uses each component's reported offset and checked
extent; compacted size retains its checked layout. A fully zeroed, aligned area
with zero `XSTATE_BV`, `XCOMP_BV` and reserved header bytes is an explicit
architectural initial image. One live predicate requires a configured form,
live `CR4.OSXSAVE` and exact live XCR0; both early user construction and the
final crossing ask that same predicate.

**Alternatives and current evidence.** Trusting the remembered form at the
early door was rejected because it disagrees with the final hardware boundary.
Saving once merely to initialize a slot was rejected because architectural
zeros already encode initial state. Host tests cover partial masks and malformed
standard offsets/extents. Source and compiler contracts bind both user doors,
and retained mutations cover component policy, slot clearing and substitution
of the remembered predicate. Fresh-slot and current hardware observations
remain PW-132 and PW-305.
