# D-187 PCI BAR discovery and function ownership are transactions

Status: Accepted (2026-08-27), amends D-119 and D-168, implements PW-201 and closes PW-202

**Context.** D-168 made a PCI function the authority for its decoded windows,
but three implementation details contradicted the unchanged-on-refusal
contract. The claim key included a configuration-register offset, so two
addresses inside one header were two apparent devices. The device registry
committed before header reads, BAR probes and permanent port claims, and I/O
windows were claimed one by one, so a later refusal could strand both kinds of
ownership. A 64-bit BAR was also probed as two separate save/write/read/restore
operations while memory and I/O decode remained enabled. Hardware therefore
never saw both halves holding all ones, and could transiently decode the probe
value.

**Decision — configuration probing is one restored transaction.** The pure
algorithm lives in `pci-config`; `wost` holds the CF8/CFC pair once and performs
the cycles it requests. The algorithm first saves the command register and
every BAR allowed by the header layout. It rejects an announced wide BAR in
the final slot before writing anything, clears both decode-enable bits with a
sixteen-bit command value, writes both halves of a wide BAR before reading
either, and restores modified BARs in reverse order before restoring command
last. A write is treated as possibly effective even when its adapter reports
failure. Rollback therefore attempts every remaining restore, and a refused
restore is distinct from the initiating access error. Status bits are never
echoed into their write-one-to-clear halfword.

**Decision — a claim has one identity and one commit point.** `Address` exposes
its canonical bus/device/function identity with the register offset cleared.
A bounded `ClaimRegistry` canonicalizes again at its boundary, checks the
duplicate and capacity conditions, runs one fallible preparation closure, and
records only the value that closure successfully returns. The kernel holds the
registry's token-bearing lock across that operation. Preparation probes the
header and asks the port registry for every I/O window in one `reserve_all`
operation; after a successful permanent port reservation no fallible work
remains. There is still no release and no half-claimed interval.

**Decision — pure mutations name exact host tests.** D-119 assumed every
meaningful defect needed a QEMU boot. Transient configuration ordering is both
hardware-free logic and something QEMU can permissively hide. A mutation may
now name an exact host package and test. The runner accepts only that test's
own `FAILED` verdict; a compile failure or failure elsewhere remains
inconclusive. Machine mutations continue to boot and correlate the exact
machine-check name. Restoration of the source precedes either verdict.

**Rationale.** Disabling decode and restoring command last are the ordering
constraints the PCI protocol needs, while the pure event interface makes every
intermediate state observable without putting a fake device in the kernel.
Putting the registry commit after a closure mechanically enforces rollback for
every future preparation exit instead of relying on a carefully placed
assignment. `PortSpace::reserve_all` already owns the atomic bitmap operation;
the new `wost` wrapper preserves that all-or-none result as typed `PortSpan`s.

**Alternatives.** Serializing two old per-dword probes was rejected because a
lock cannot make their restored interval a simultaneous wide probe. Leaving
decode enabled was rejected because emulator tolerance is not a device
contract. Releasing an early device or port claim on error was rejected
because the registries intentionally have no release and adding one would
create the lifetime race D-138 avoided. Permanently poisoning a failed claim
was safe but contradicted the documented unchanged-on-error API and prevented
a later corrected driver from trying again.

**Consequences.** Host fake configuration tests assert the exact command/BAR
trace, inject a refusal at every access after mutation, continue after restore
failures, and cover malformed masks and final-slot wide BARs. A claim-registry
test injects failure at every preparation edge, and `PortSpace` places a
conflict at every requested-span index; neither may retain an earlier claim.
QEMU still discovers the NVMe controller's 64-bit BAR and boots storage. Its
machine checks prove two offsets cannot claim one bridge twice and a claimed
device's whole I/O window refuses bare access. Ten new persistent mutations
cover the pure ordering/rollback and transaction properties; the two changed
machine mutations were also driven red. PW-201 is `REQUIRED`: the
implementation is established, and the current VMware close must still observe
the controller BAR named by its register row.
