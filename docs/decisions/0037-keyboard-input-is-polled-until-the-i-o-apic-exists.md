# D-37 Keyboard input is polled until the I/O APIC exists

Status: Accepted (2026-08-20)

**Context.** The i8042 works, but its interrupt has nowhere legal to go.
IRQ 1's two routes to the CPU are the legacy 8259 pair — silenced in slice 2,
an invariant `interrupt::enable` asserts and a self-test pins — and the I/O
APIC, a memory-mapped device page behind the same missing page tables that
forced x2APIC (D-33). Slice 3 just gave the kernel a 100 Hz tick and an idle
loop that wakes on it.

**Options.**

- *Unmask IRQ 1 on the 8259s.* Interrupt-driven input today, at the cost of
  reopening the controller slice 2 existed to close: the `is_silenced`
  precondition and its self-test both weaken, a PIC-specific end-of-interrupt
  and spurious-IRQ-7 path get written, and all of it is deleted when the I/O
  APIC arrives.
- *Bring up the I/O APIC now.* The right final routing, but it needs a mapped
  device page, which needs page tables, which is Phase 3's entire subject —
  the CPU phase would become a memory phase, the exact trade D-33 refused.
- *Poll from the tick-woken idle loop.* No interrupt routing at all: the
  controller is configured to raise nothing, and each tick's wake drains it.
- *Defer the keyboard to Phase 3 wholesale.* Forfeits the phase's visible
  definition-of-done moment for no architectural gain — the driver and
  decoder are identical either way.

**Decision.** Poll. The i8042's configuration byte has both interrupt enables
cleared — the choice is enforced at the device, not assumed — and translation
cleared with them, so the decoder receives raw set 2. The idle loop drains
every pending byte per wake, batched eight events per lock hold, delivering
outside the lock.

**Rationale.** Latency is bounded by one tick: ten milliseconds against a
human perception threshold several times that, and against a keyboard that
buffers an order of magnitude more than a tick of typing. Nothing is thrown
away later — when Phase 3 pages the I/O APIC in, the polled call site in the
idle loop retires and the driver, decoder, and delivery path carry over
unchanged. It is the same deferral shape as D-33: the modern mechanism waits
for page tables, the phase ships on what is reachable.

**Consequences.**

- The driver lives entirely outside `wost`, like the UART and PIT drivers
  before it — ports have been safe since the claim registry — and the slice
  adds zero new `unsafe`.
- Input is dead if the timer declined: no tick, no wake, no drain. Accepted —
  the self-test would already be red on the timer checks, and a machine whose
  PIT and APIC both lie has larger problems than typing.
- Echoed characters ride the console lock from mainline context only; the
  trap-context prohibition of D-31/D-34 is never in play.
- Backspace echoes nothing until the console grows a cursor-retreat
  primitive with a real second caller (Phase 7's shell).
