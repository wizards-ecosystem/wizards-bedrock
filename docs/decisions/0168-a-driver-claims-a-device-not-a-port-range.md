# D-168 A driver claims a device, not a port range

Status: Accepted (2026-08-25)

**Context.** D-77 closed half an authority debt and named the other half. A
port claim proves **exclusivity** — nobody else drives these ports — and says
nothing about **entitlement**, about whether the claimant is the driver for
whatever is behind them. The port registry cannot answer that, because a port
range is not a device: it has no identity, no vendor, and no way to be asked
what it is.

`014` lists it as the first of the phase's open decisions: *does a device claim
replace a port claim, or sit above it?*

**Decision.** It contains it. A driver claims a **function** — a bus, device
and function triple — and the claim carries every window that function decodes
with it. A bare port claim inside a claimed device's I/O window is then refused
by the port registry itself, because the device already holds those ports.

Not "authorises claiming", which is the alternative `014` names, because that
leaves two mechanisms and a gap between them: a driver could hold a device
capability and never claim the ports, or claim ports it has no device for, and
nothing would connect the two. One object holding both makes D-138's
permanence a property of the device rather than of every range separately.

**Claims are permanent**, exactly as port claims are. There is no release, so
there is no window in which a device is half-claimed, and a refusal strands
nothing: the holder is unaffected and the refused caller has taken nothing.

**Rationale, and five things the specification decides rather than taste.**

**CF8/CFC, not ECAM.** ECAM needs an MCFG table; QEMU's i440fx publishes none
and VMware emulates a 440BX. The 1992 mechanism is the only one guaranteed on
**both** targets D-18 makes this project answer to, and it reaches every byte a
read-only driver needs. An ECAM path later is purely additive (`012` §2).

**Enumeration must recurse, and the check machine had to be given something to
recurse into.** q35 as QEMU ships it has no PCI-to-PCI bridge, so a kernel that
scanned bus zero and stopped would have found all six functions, reported a
plausible number, and passed — while being unable to find a controller on a
real machine, because VMware places functions behind `pciBridge0`–`pciBridge7`.
The check machine now gets a `pci-bridge`, and the survey counts **buses**, not
only functions: the function count says nothing about descent and the bus count
says everything.

**A bridge has two base address registers, not six.** A type 1 header puts its
primary, secondary and subordinate bus numbers where a device's third register
would be. A claim that read six would decode those bus numbers as a window and
then *probe* it — writing all ones into the register that says which bus is
behind this bridge. The probe restores what it found, so nothing downstream
would necessarily notice; the check asserts the claim decodes two.

**The window size is complemented in the register's own width.** The first
version had one `window_size` taking "pass all ones for a narrow register",
and the I/O path then complemented a thirty-two-bit mask in sixty-four bits and
answered eighteen quintillion. Caught by a doctest on its first run. It is two
functions now — `memory_window_size` with an explicit `wide`, and
`io_window_size` returning a `u16` — because a width you must remember to pass
correctly is a width that will be passed wrongly.

**Bus master is set by the thing that holds the claim.** Without it a
controller cannot DMA, queues never advance, and *there is no error anywhere to
read* (`012` §2). It is the most expensive bit in this subsystem to forget, so
the claim owns it rather than a driver that might not.

**The lock check earned its keep on the first boot.** `pci::init` took this
module's guard and then called `port::claim_pair`, which takes the port
registry's — leaf under leaf, which is exactly what `lock_leaf` forbids and
what D-121's machine check watches for. It went red on the first run, naming
both sites. The claim happens outside the guard now.

**Consequences.**

- `libs/pci-config`: address encoding, header offsets, class codes, header
  types, BAR decoding, window sizing. Fifteen host tests and three doctests,
  five deliberate breaks, all red.
- `wost::arch::pci`: the two ports, claimed as a pair (D-138), and the probe
  that restores what it found. A register left holding `0xFFFF_FFFF` is a
  device claiming four gigabytes at the top of memory, and the next thing to
  enumerate believes it.
- `wost::arch::port::claim_span`: a run whose length the *machine* supplies.
  `claim_block` wants a constant, which is right for a PIT with three ports and
  a UART with eight; a device's I/O window is whatever the probe says.
- `kernel::pci`: an iterative walk over a bus worklist, each bus visited once —
  recursion would put a bridge topology on a thirty-two-kilobyte stack, and a
  bridge that names itself as its own secondary bus is a cycle.
- Three machine checks, five breaks, all red.
- **The `Device` is a kernel-side capability, not yet a handle.** It is
  obtainable only from the registry, exactly as `Port<T>` is, which is the same
  discipline one level up. Slice 8 makes it resolvable internally through devfs
  but does not create a handle. A user-visible Device handle is explicitly
  post-compiler-gate work in D-184's register, not pre-Wizzy foundation debt.
