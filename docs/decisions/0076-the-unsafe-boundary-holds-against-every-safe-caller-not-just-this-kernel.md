# D-76 The unsafe boundary holds against every safe caller, not just this kernel

Status: Accepted (2026-08-23) (amended by D-77)

**Context.** A soundness review of `wost` asked D-13's question the hard way:
not "does the kernel misuse these APIs" but "*could* any safe caller". Five
holes survived their own refutation, every one a safe path to undefined
behaviour or to hardware state the type system claimed impossible. One was a
plain bug: the fabricated first frame was seven words, so `thread_start`
entered with the stack pointer *at* the page-aligned top — `rsp ≡ 0 (mod 16)`
where the ABI promises 8 — and every non-boot thread ran its whole life
shifted, symptomless only because the kernel builds without SSE. The comment
justifying the seven miscounted the `ret`.

**Decision.** Five contracts hardened, each behind a check that went red first:

- The then-current `mm::read_physical` **copied instead of lending**. The `&'static [u8]` it
  returned could cover memory the kernel itself writes — its own `.data`, the
  boot stack, the framebuffer — and a reference over concurrently written
  memory is an aliasing lie whatever the kind filter says. Volatile reads into
  a caller's buffer are defined for every region the map describes, so the
  kind rules stayed what D-64 set and the lifetime lie was gone. D-77 removed
  this general capability entirely and replaced it with ACPI-only authority.
- `AddressSpace::set` **refuses to re-point a present entry** (identical
  rewrites stay legal — the direct map's outward rounding produces them).
  Silent overwrite let safe code revoke what a `StackArena` or `DeviceMapping`
  is evidence of.
- `DeviceMapping` **carries the root of the space that minted it**, and
  `claim` compares it against the active root. Window addresses collide
  across spaces, so a stale claim from a dropped space could cover whatever
  the installed space put at the same address — two `MmioRegion`s, one device.
- `activate` **walks the whole direct map** in the candidate tables — every
  `is_direct_mapped` region, writable and never-executable at its own frame —
  before loading `CR3`. Every frame access after the switch is raw arithmetic
  against that mapping; the walk is what turns "the kernel surely mapped it"
  into a refusal that is still a `Result`. Stepping by leaf spans keeps it to
  hundreds of walks.
- `switch_from_trap` **accepts only the frame the live handler was entered
  with** (`ForeignFrame` otherwise). The dispatcher records the frame it
  passes, and pointer identity — parked and restored across switches exactly
  like the trap depth — makes it evidence. D-77 additionally made frame state
  private and read-only to safe handlers. `fabricate`'s frame gained its eighth
  word, and the machine now checks a spawned thread's actual stack alignment,
  a claim broken from slice 1 that no register test could see.

Three smaller closures rode along: `boot::enter` is once-only (a second call
could mint a second `Framebuffer` over the same device), the PAT was rewritten
*before* `activate` so the `CR3` load was the flush the manual asks for, and
three `SAFETY` comments now say true things (`MmioRegion: Send` rests on
`!Sync`, not a `&mut` discipline it never had; `breakpoint` asserts the table
it claimed; `apic::write` accounts for its two zero-writes).

**Rationale.** D-13's containment claim is only worth its price if the safe
boundary is sound against adversarial safe code — that is the property the
whole crate split buys. Amends D-45, D-49, D-51, D-54 and D-64, whose
mechanisms changed shape; their decisions stand.

**Consequences.**

- Machine checks at this checkpoint were `a thread starts aligned`, `a
  differing remap is refused`, and `a stale device claim is refused`; the
  direct-map walk was broken once by skipping a region and refused the switch
  on cue. The checkpoint had 75 on-machine checks; D-77 raises the current
  total.
- **Known residue, deliberate:** `AddressSpace::map` still accepts any
  physical address without ownership evidence, so a hostile safe caller could
  alias a live frame into a fresh space before activating it. Closing that
  means mappings take owned frames — D-44's type discipline applied to the
  mapping API — which reshapes `map`'s signature and waits for Phase 5's
  memory objects rather than being bolted on here.
