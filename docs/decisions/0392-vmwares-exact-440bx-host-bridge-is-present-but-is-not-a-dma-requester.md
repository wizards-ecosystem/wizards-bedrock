# D-392 VMware's exact 440BX host bridge is present but is not a DMA requester

Status: Accepted, 2026-10-12. Amends D-390; advances PW-594 and PW-596.

**Context.** The first source-bound VMware run of D-390 failed closed before
memory initialization at `00:00.0`: command `0x0006` remained `0x0006` after
the kernel wrote `0x0002`. VMware's emulated Intel 440BX host bridge reports a
read-only bus-master-looking bit even though that host bridge is not an
endpoint requester. Treating every refused clear as fatal made the generic
rule safe but made the qualified VMware machine unbootable. Ignoring the
readback, exempting all bridges or changing only the early fence would instead
create an unproved DMA and later VT-d inventory bypass.

**Decision.** Strict clearing remains the default and the complete QEMU TCG
policy. VMware may classify exactly one function as a non-requester only when
all of the following agree: the accepted unique `WZDH`, the exact CPU-resident
VMware identity, segment-zero BDF `00:00.0`, identity `0x7190_8086`, class
`06:00:00` with revision ignored, command before and readback both exactly
`0x0006`, and the attempted command exactly `0x0002`. After the refused write,
the kernel re-reads both identity and class before the entry proof can exist.
The classifier returns a private, non-copyable token; the PCI topology consumes
that token to retain the function as present while excluding it from the
requester inventory. Later whole-unit coverage and exact-requester lookup use
that requester inventory rather than raw function presence. A refused clear
never increments the count of requesters actually disabled.

The entry transcript names the exact retained function and classification.
Both VMware self-test and shipped selectors require that record exactly once;
QEMU self-test, shipped and degraded selectors instead require the exact
strict-policy record. No other platform, BDF, identity, class, command state or
changed readback inherits this exception.

**Proof required.** The pure PCI suite accepts all 440BX revision bytes but
rejects every near miss in platform, BDF component, vendor/device identity,
class/subclass/programming interface, command-before and command-after state.
It must also reject identity or class replacement after the write and prove
that the accepted token leaves the bridge present but not a requester. Source
contracts must bind the same policy through entry, memory initialization and
the later topology snapshot, and must require requester—not presence—queries
in both VT-d inventory and whole-unit coverage. Exact QEMU and VMware
transcript tests, mutations 1,840 through 1,865, the complete gate and fresh
self-test and shipped VMware qualifications must all pass before PW-594 or
PW-596 closes.

**Alternatives.** A class-wide host-bridge exemption lets an unrelated or
spoofed function bypass the fence. Trusting only the fixed BDF ignores mutable
identity and command state. Recording the exception only in the early report
would reintroduce the same function as a requester during VT-d coverage.
Calling the refused write a successful disable would overstate the measured
boundary. Rejecting VMware entirely remains fail-closed but discards an already
qualified release platform without improving QEMU or bare-metal safety.
