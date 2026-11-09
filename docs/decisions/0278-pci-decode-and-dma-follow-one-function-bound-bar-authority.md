# D-278 PCI decode and DMA follow one function-bound BAR authority

Status: Accepted, 2026-09-04. Amends D-187, D-197, D-251 and D-265; advances PW-536 and PW-537.

**Context.** The storage path enabled memory decoding before page-rounded RAM
exclusion and mapped a raw physical range against the page-table format's
static 52-bit ceiling. Bus mastering then required only the PCI function claim
and a copied NVMe observation. Safe callers could therefore map reserved
physical-address bits, enable a changed BAR, or pair one disabled controller
with another function.

**Decision.** Physical MMIO admission first decodes the running CPU's
`MAXPHYADDR`, validates the complete page-rounded extent against that width and
the immutable boot memory map, and produces a linear range token. A live PCI
function binds that token to its opaque registry generation, exact BAR slot and
decoded BAR value. The PCI mapping door consumes this authority. Memory decode
is enabled only after the mapped token returns to the same function and a fresh
configuration read proves that BAR identity unchanged.

The resulting register region retains the function and BAR binding. The only
bus-master transition consumes a fresh, non-constructible observation borrowed
from that exact region after reading `CC.EN = 0` and `CSTS.RDY = 0`; the borrow
prevents safe register access between observation and transition. Unqualified
hardware adapters that enable memory decode or bus mastering are removed. Pure
configuration transactions remain host-testable, but holding a function claim
alone is no longer enough to reach either platform write.

BAR probing first clears and reads back I/O decode, memory decode and inherited
bus mastering before writing any BAR. Its staged result retains the original
command and the function-bound restoration sublease; memory decode borrows that
same state while keeping bus mastering clear, and only the later quiescence
authority may add it. Unsupported memory BAR type encodings are rejected before
the first probe write, and the live identity comparison includes those type
bits rather than only the address.

NVMe preparation also carries its mapped owner through every fallible register
access. Before disabled state is proven a refusal returns a quarantine owner;
afterward it either retires the BAR exactly or returns the still-mapped owner.
The selector retains any uncertain mapping with its PCI lease instead of
reporting an ordinary clean retirement or dropping the sole cleanup route.

**Alternatives and proof obligation.** Passing an address, a boolean, or a
copyable controller-state record was rejected because each lets the caller
restate the conclusion. Checking only the unrounded BAR was rejected because a
page mapping reaches bytes outside it. Treating the PTE encoding's maximum as
the processor's maximum was rejected because reserved physical bits can fault
only when first accessed. PW-536 and PW-537 close only after width and range
properties, hostile configuration/MMIO traces, compile-time door inventory,
durable identity/ordering/quiescence mutations and restored machine boots all
pass. No native BAR observation is claimed by this decision.

The implementation now carries one chain from `PhysicalAddressWidth` through a
page-rounded `AdmittedDeviceRange`, exact `BarAuthority`, inaccessible mapping,
fresh BAR identity readback and `MappedBar`, then through typed preparation,
fresh `CC`/`CSTS` classification and a borrowed `NvmeBusMasterAuthority` to the
sole bus-master write. Focused `pci-config` and `nvme` suites pass 72 plus two
doctests and 33 plus two doctests respectively; the memory-map, paging and
CPUID suites and the compile/source door contracts also pass. Inherited-BME,
staged-owner, clean-rollback, BAR-type, identity, preparation-quarantine and
retirement-owner breaks reddened their focused witnesses before restoration,
and retained equivalents are registered. Current release-machine mutation
runs, PW-206's causal refusal, the live `MAXPHYADDR` marker break, VMware and
the complete mutation sweep remain open, so PW-536 and PW-537 remain `READY`.
