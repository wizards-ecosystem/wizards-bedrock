# D-37 Keyboard input is polled until the I/O APIC exists

Status: Accepted, 2026-08-20.

**Decision.** Poll the i8042 from the tick-woken idle loop. Its
configuration byte has both interrupt enables cleared, so the choice is
enforced at the device, not assumed. Translation is cleared with them, so
the decoder receives raw set 2. The idle loop drains every pending byte per
wake, batched eight events per lock hold, delivering outside the lock.

**Rationale.** IRQ 1 can reach the CPU only through the silenced 8259 pair,
whose `is_silenced` invariant `interrupt::enable` asserts, or the I/O APIC,
a device page behind the missing page tables of D-33. One tick bounds
latency to ten milliseconds, well under human perception. When Phase 3 pages
the I/O APIC in, only the polled call site retires; the driver, decoder and
delivery path carry over unchanged.

**Consequences.**

- The driver lives entirely outside `wost`, like the UART and PIT drivers
  before it (ports are safe since the claim registry), and adds zero new
  `unsafe`.
- Input is dead if the timer declined: no tick, no wake, no drain. Accepted,
  since the self-test would already be red on the timer checks.
- Echoed characters take the console lock from mainline context only; the
  trap-context prohibition of D-31/D-34 never applies.
- Backspace echoes nothing until the console grows a cursor-retreat
  primitive with a second caller (Phase 7's shell).
