# D-351 Fixed devices are claimed by name, not by port number

Status: Accepted, 2026-09-29. Amends D-138; closes PW-561.

**Decision.** `wost` exposes one typed door per known device: `debug_exit`,
`i8042`, `pit`, `com1` and `pci_configuration`. Each names its own ports and
returns the exact `Port` or `PortBlock` that device needs. The device port
numbers live only in `wost`, the platform crate. The kernel drivers ask for
the device by name. A self-test-only `is_claimed` probe answers whether a
driver's ports are already spoken for without minting a usable claim, so the
exclusivity machine checks need no raw claim. A source contract parses every
production kernel source except the self-test harness and refuses any call to
a raw `port::claim`, `claim_pair`, `claim_block` or `claim_block_pair`; the
dynamic PCI I/O BAR path keeps `claim_spans`, which a function claim already
gates. The raw doors stay public so the self-test harness can probe the
registry, which is the one place that still names a raw claim.

**Rationale.** The exclusive registry kept two callers from holding the same
ports, but nothing said that the caller naming 0x60 was the keyboard driver:
any code with the port module in scope could reserve any device's ports.
Mutation 1,533 reverts the debug-exit driver to `port::claim::<u32>(0xF4)` and
fails the contract, which names the offending file and call.

**Alternatives.** Making the raw doors private was rejected because the
self-test harness probes the registry through them to prove exclusivity, and a
private door cannot be reached from the kernel crate. A runtime capability
token per device was rejected as heavier than the fixed set of compile-time
doors a from-scratch kernel needs. Leaving the numbers in the driver modules
was rejected because it kept the entitlement gap the finding named.
