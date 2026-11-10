# D-37 Keyboard input is polled until the I/O APIC exists

Status: Accepted, 2026-08-20.

**Context.** The i8042 works, but its interrupt has nowhere legal to go.
IRQ 1 reaches the CPU through the legacy 8259 pair, silenced in slice 2 (an
invariant `interrupt::enable` asserts and a self-test pins), or through the
I/O APIC, a memory-mapped device page behind the same missing page tables
that forced x2APIC (D-33). Slice 3 just gave the kernel a 100 Hz tick and an
idle loop that wakes on it.

**Options.**

- *Unmask IRQ 1 on the 8259s.* Interrupt-driven input today, but it reopens
  the controller slice 2 closed. The `is_silenced` precondition and its
  self-test both weaken, and a PIC-specific end-of-interrupt and
  spurious-IRQ-7 path get written, all deleted when the I/O APIC arrives.
- *Bring up the I/O APIC now.* The right final routing, but a mapped device
  page needs page tables, Phase 3's entire subject. The CPU phase would
  become a memory phase, the trade D-33 refused.
- *Poll from the tick-woken idle loop.* No interrupt routing at all: the
  controller raises nothing, and each tick's wake drains it.
- *Defer the keyboard to Phase 3 wholesale.* Forfeits the phase's visible
  definition-of-done moment for no architectural gain; the driver and
  decoder are identical either way.

**Decision.** Poll. The i8042's configuration byte has both interrupt
enables cleared, so the choice is enforced at the device, not assumed.
Translation is cleared with them, so the decoder receives raw set 2. The
idle loop drains every pending byte per wake, batched eight events per lock
hold, delivering outside the lock.

**Rationale.** One tick bounds latency: ten milliseconds, against a human
perception threshold several times that and a keyboard buffering an order
of magnitude more than a tick of typing. Nothing is thrown away. When
Phase 3 pages the I/O APIC in, only the polled call site in the idle loop
retires; the driver, decoder and delivery path carry over unchanged. This
is D-33's deferral shape: the modern mechanism waits for page tables, the
phase ships on what is reachable.

**Consequences.**

- The driver lives entirely outside `wost`, like the UART and PIT drivers
  before it (ports are safe since the claim registry), and the slice adds
  zero new `unsafe`.
- Input is dead if the timer declined: no tick, no wake, no drain. Accepted,
  since the self-test would already be red on the timer checks. A machine
  whose PIT and APIC both lie has larger problems than typing.
- Echoed characters take the console lock from mainline context only; the
  trap-context prohibition of D-31/D-34 never applies.
- Backspace echoes nothing until the console grows a cursor-retreat
  primitive with a second caller (Phase 7's shell).
