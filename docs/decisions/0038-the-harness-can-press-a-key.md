# D-38 The harness can press a key

Status: Accepted, 2026-08-20. Extends D-28.

**Context.** D-28's self-test only observes what the machine does to
itself: a self-interrupt proves the interrupt path, its own tick the timer.
An input device's defining behaviour starts outside the machine. Without an
outside hand, the keyboard path (controller, buffering, decoding) ships with
its main road untested.

**Options.**

- *Host tests plus human eyes.* Leaves the only hardware-facing half
  unasserted; regressions surface days later as "typing stopped working" on
  a VMware boot.
- *QEMU's monitor `sendkey` via `human-monitor-command`.* Works, but QEMU
  releases the key on its own hold-time schedule. The check asserts the
  break code's arrival and order, so the timing must be ours.
- *A QMP crate.* A dependency for three fixed message exchanges, against
  D-25's precedent that a hand-rolled hundred lines beats a supply-chain
  edge.
- *A hand-rolled QMP client sending `input-send-event`.* Chosen.

**Decision.** `cargo xtask test-boot` binds a loopback `TcpListener` on an
ephemeral port before spawning QEMU and passes `-qmp tcp:{addr}`, so QEMU
dials out: no fixed port to collide, no bind-then-connect race, no retry
loop. A forwarding thread pipes serial, still passing every line to the
console and flushing a dying kernel's unterminated last line. It watches for
the line the kernel prints just before its bounded wait, `selftest: awaiting
injected keystroke`, a two-file contract kept in step by comment like the
exit codes. The harness then sends one command with an explicit press and
release of `A`. The kernel's check requires the make, then the break, to
decode in order.

**Rationale.** Keys wait for that line instead of being sprayed from
power-on. Early keystrokes land in firmware setup prompts or the bootloader
menu, and the self-test's own controller commands (config readback, echo,
set readback) must never interleave with injected bytes; the check's comment
records that ordering. The verdict stays the kernel's. A QMP failure prints
its cause and retires the injector. The kernel's doubly bounded wait (a tick
budget, plus an iteration cap so a dead timer cannot turn red into a hang)
then fails the check, with the sixty-second kill behind everything. Replies
are classified by token, not parsed as JSON. That suffices: both commands
are fixed strings, the response set is tiny, and unsolicited `SHUTDOWN`
events, which arrive on this socket when the exit device fires, are just
skipped.

**Consequences.**

- `test-boot`'s serial output now passes through a pipe. Behaviour is
  preserved line-for-line, and the change is intended.
- Injection is always on for `test-boot`: a check that only sometimes gets
  its stimulus only sometimes can fail.
- The QMP failure path was broken once on purpose (the `-qmp` argument
  deleted): the run went red on the keystroke check instead of hanging.
- Real hardware has no QMP or injector, so the check would go red there,
  correctly, since nothing outside presses a key. The self-test feature never
  ships in an image (D-28), so a real boot is unchanged.
