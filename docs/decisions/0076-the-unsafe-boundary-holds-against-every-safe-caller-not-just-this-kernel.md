# D-76 The unsafe boundary holds against every safe caller, not just this kernel

Status: Accepted, 2026-08-23. Amended by D-77.

**Decision.** D-13's question is asked of every safe caller, not only this
kernel: no safe path may reach undefined behaviour or hardware state the type
system claims impossible. Five contracts hardened, each behind a check that
first went red:

- The then-current `mm::read_physical` **copied instead of lending**. A
  `&'static [u8]` could cover memory the kernel writes, and a reference over
  concurrently written memory is an aliasing lie whatever the kind filter
  says, so D-64's kind rules stayed and the lifetime lie went. D-77 removed
  this general capability for ACPI-only authority.
- `AddressSpace::set` **refuses to re-point a present entry**; identical
  rewrites, which the direct map's outward rounding produces, stay legal.
  Silent overwrite let safe code revoke what a `StackArena` or `DeviceMapping`
  is evidence of.
- `DeviceMapping` **carries the root of the space that minted it**, and
  `claim` compares it with the active root, because window addresses collide
  across spaces.
- `activate` **walks the whole direct map** in the candidate tables before
  loading `CR3`: every `is_direct_mapped` region, writable and
  never-executable at its own frame. Every frame access after the switch
  relies on that mapping, so the walk makes it a refusal that is still a
  `Result`.
- `switch_from_trap` **accepts only the frame the live handler was entered
  with** (`ForeignFrame` otherwise); pointer identity, parked and restored
  across switches like trap depth, makes it evidence. D-77 also made frame
  state private and read-only to safe handlers.

One plain bug: the fabricated first frame was seven words, so `thread_start`
entered with `rsp ≡ 0 (mod 16)` where the ABI promises 8. That seven-word
frame misaligned every new thread; `fabricate`'s frame gained its eighth word,
and the machine checks a spawned thread's actual stack alignment.

`boot::enter` is once-only, because a second call could mint another
`Framebuffer` over the device. The PAT is rewritten *before* `activate`, so the
`CR3` load is the flush the manual asks for. `MmioRegion: Send` rests on
`!Sync`.

**Rationale.** D-13's containment claim is worth its price only if the safe
boundary holds against adversarial safe code, which the crate split buys.
Amends D-45, D-49, D-51, D-54 and D-64, whose mechanisms changed shape; their
decisions stand.

**Consequences.**

- The checkpoint had 75 on-machine checks; D-77 raises the current total.
- **Known residue, accepted:** `AddressSpace::map` still accepts any physical
  address without ownership evidence, so a hostile safe caller could alias a
  live frame into a fresh space before activating it. Closing it means
  mappings take owned frames (D-44's type discipline applied to mapping),
  which reshapes `map`'s parameters and waits for Phase 5's memory objects.
