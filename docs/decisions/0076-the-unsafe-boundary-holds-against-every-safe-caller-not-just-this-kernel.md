# D-76 The unsafe boundary holds against every safe caller, not just this kernel

Status: Accepted, 2026-08-23. Amended by D-77.

**Context.** A soundness review of `wost` asked D-13's question the hard way:
not "does the kernel misuse these APIs" but "*could* any safe caller". Five
holes survived refutation, each a safe path to undefined behaviour or to
hardware state the type system claimed impossible. One was a plain bug: the
fabricated first frame was seven words, so `thread_start` entered with the
stack pointer *at* the page-aligned top (`rsp ≡ 0 (mod 16)` where the ABI
promises 8). Every non-boot thread ran shifted, symptomless only because the
kernel builds without SSE. The comment justifying the seven miscounted the
`ret`.

**Decision.** Five contracts hardened, each behind a check that first went
red:

- The then-current `mm::read_physical` **copied instead of lending**. The
  `&'static [u8]` it returned could cover memory the kernel writes (its own
  `.data`, the boot stack, the framebuffer), and a reference over concurrently
  written memory is an aliasing lie whatever the kind filter says. Volatile
  reads into a caller's buffer are defined for every region the map describes,
  so D-64's kind rules stayed and the lifetime lie went. D-77 removed this
  general capability for ACPI-only authority.
- `AddressSpace::set` **refuses to re-point a present entry**; identical
  rewrites, which the direct map's outward rounding produces, stay legal.
  Silent overwrite let safe code revoke what a `StackArena` or `DeviceMapping`
  is evidence of.
- `DeviceMapping` **carries the root of the space that minted it**, and
  `claim` compares it with the active root. Window addresses collide across
  spaces, so a stale claim from a dropped space could cover whatever the
  installed space put there: two `MmioRegion`s, one device.
- `activate` **walks the whole direct map** in the candidate tables before
  loading `CR3`: every `is_direct_mapped` region, writable and
  never-executable at its own frame. Every frame access after the switch is
  raw arithmetic against that mapping; the walk turns "the kernel surely
  mapped it" into a refusal that is still a `Result`. Stepping by leaf spans
  keeps it to hundreds of walks.
- `switch_from_trap` **accepts only the frame the live handler was entered
  with** (`ForeignFrame` otherwise). The dispatcher records the frame it
  passes; pointer identity, parked and restored across switches like trap
  depth, makes it evidence. D-77 also made frame state private and read-only
  to safe handlers. `fabricate`'s frame gained its eighth word, and the
  machine checks a spawned thread's actual stack alignment, broken since slice
  1 and invisible to any register test.

Three smaller closures. `boot::enter` is once-only (a second call could mint
another `Framebuffer` over the device). The PAT was rewritten *before*
`activate`, so the `CR3` load was the flush the manual asks for. Three
`SAFETY` comments now say true things: `MmioRegion: Send` rests on `!Sync`,
not a `&mut` discipline it never had; `breakpoint` asserts the table it
claimed; `apic::write` accounts for its two zero-writes.

**Rationale.** D-13's containment claim is worth its price only if the safe
boundary holds against adversarial safe code, which the crate split buys.
Amends D-45, D-49, D-51, D-54 and D-64, whose mechanisms changed shape; their
decisions stand.

**Consequences.**

- Machine checks at this checkpoint were `a thread starts aligned`, `a
  differing remap is refused`, and `a stale device claim is refused`. The
  direct-map walk, broken once by skipping a region, refused the switch on
  cue. The checkpoint had 75 on-machine checks; D-77 raises the current total.
- **Known residue, accepted:** `AddressSpace::map` still accepts any physical
  address without ownership evidence, so a hostile safe caller could alias a
  live frame into a fresh space before activating it. Closing it means
  mappings take owned frames (D-44's type discipline applied to mapping),
  which reshapes `map`'s parameters and waits for Phase 5's memory objects.
