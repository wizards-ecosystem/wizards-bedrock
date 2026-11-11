# D-168 A driver claims a device, not a port range

Status: Accepted, 2026-08-25.

**Decision.** A device claim contains port claims. A driver claims a
**function**, a bus, device and function triple, and the claim carries every
window that function decodes. A bare port claim inside a claimed device's I/O
window is refused by the port registry itself, because the device already holds
those ports. **Claims are permanent**, as port claims are: there is no release,
so no device is ever half-claimed, and a refusal strands nothing.

A device claim does not merely authorise claiming ports, because that leaves two
mechanisms and a gap between them: a driver could hold a device and never claim
its ports, or claim ports it has no device for. One object holding both makes
D-138's permanence a property of the device rather than of every range.

The specification decides the rest:

- **CF8/CFC, not ECAM.** ECAM needs an MCFG table; QEMU's i440fx publishes none
  and VMware emulates a 440BX. CF8/CFC is the only mechanism guaranteed on both
  targets D-18 names, and it reaches every byte a read-only driver needs. An
  ECAM path later is purely additive (`012` §2).
- **Enumeration recurses**, because VMware places functions behind
  `pciBridge0` to `pciBridge7`. The check machine gets a `pci-bridge`, and the
  survey counts **buses**, not only functions, because the function count says
  nothing about descent. The walk is iterative over a bus worklist, each bus
  visited once: recursion would put a bridge topology on a thirty-two-kilobyte
  stack, and a bridge that names itself as its own secondary bus is a cycle.
- **A bridge has two base address registers, not six.** A type 1 header puts
  its bus numbers where a device's third register would be, and probing that as
  a window writes all ones into the register that says which bus is behind the
  bridge. The check asserts the claim decodes two.
- **The window size is complemented in the register's own width**:
  `memory_window_size` takes an explicit `wide`, and `io_window_size` returns a
  `u16`, because a width you must remember to pass is a width that will be
  passed wrongly.
- **Bus master is set by the thing that holds the claim.** Without it a
  controller cannot DMA, queues never advance, and there is no error anywhere
  to read (`012` §2).
- `pci::init` claims the configuration ports outside its own guard: taking the
  port registry's guard under it was leaf under leaf, which `lock_leaf` forbids
  and D-121's machine check caught.

**Rationale.** A port claim (D-77) proves exclusivity and says nothing about
entitlement, and the port registry cannot answer that, because a port range has
no identity, no vendor and no way to be asked what it is.

**Consequences.**

- `libs/pci-config`: address encoding, header offsets, class codes, header
  types, BAR decoding, window sizing, host-tested.
- `wost::arch::pci` claims the two ports as a pair (D-138), and its probe
  restores what it found: a register left holding `0xFFFF_FFFF` is a device
  claiming four gigabytes at the top of memory.
- `wost::arch::port::claim_span` claims a run whose length the machine
  supplies; `claim_block` keeps a constant length for fixed devices.
- Three machine checks, five breaks, all red.
- **The `Device` is a kernel-side capability, not yet a handle.** It is
  obtainable only from the registry, exactly as `Port<T>` is. Slice 8 makes it
  resolvable internally through devfs but does not create a handle. A
  user-visible Device handle is explicitly post-compiler-gate work in D-184's
  register, not pre-Wizzy foundation debt.
