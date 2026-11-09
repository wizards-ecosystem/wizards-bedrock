# D-38 The harness can press a key

Status: Accepted (2026-08-20; extends D-28)

**Context.** D-28's self-test can only observe what the machine does to
itself: it proves the interrupt path with a self-interrupt and the timer with
its own tick, but an input device's defining behaviour starts outside the
machine. Without an outside hand, the whole keyboard path — controller,
buffering, decoding — ships with its main road untested.

**Options.**

- *Host tests plus human eyes.* Leaves the only hardware-facing half
  unasserted; regressions surface as "typing stopped working" on a VMware
  boot, days later.
- *QEMU's monitor `sendkey` via `human-monitor-command`.* Works, but the
  release is automatic on QEMU's hold-time schedule — the check wants to
  assert the break code's arrival and order, so the timing must be ours.
- *A QMP crate.* A dependency for three fixed message exchanges, against
  D-25's precedent that a hand-rolled hundred lines beats a supply-chain
  edge.
- *A hand-rolled QMP client sending `input-send-event`.* Chosen.

**Decision.** `cargo xtask test-boot` binds a loopback `TcpListener` on an
ephemeral port before spawning QEMU and passes `-qmp tcp:{addr}` so QEMU
dials out — no fixed port to collide, no bind-then-connect race, no retry
loop. Serial is piped through a forwarding thread (every line still reaches
the console; a dying kernel's unterminated last line is flushed too) which
watches for one marker the kernel prints immediately before its bounded
wait: `selftest: awaiting injected keystroke`, a two-file contract kept in
step by comment exactly as the exit codes already are. On the marker, the
harness sends one command carrying an explicit press and release of `A`; the
kernel's check requires the make and then the break to decode, in order.

**Rationale.** Marker-gating rather than spraying keys from power-on: early
keystrokes land in firmware setup prompts or the bootloader menu, and the
self-test's own controller commands (config readback, echo, set readback)
must never interleave with injected bytes — the ordering is load-bearing and
commented at the check. The verdict stays the kernel's: any QMP failure
prints its cause and retires the injector, whereupon the kernel's doubly
bounded wait (a tick budget, plus an iteration cap so a dead timer cannot
convert red into hang) fails the check, and the sixty-second kill stands
behind everything. Reply handling is token classification, not JSON parsing,
which is sufficient because both commands are fixed strings, the response
set is tiny, and unsolicited `SHUTDOWN` events — which genuinely arrive on
this socket when the exit device fires — need only to be skipped.

**Consequences.**

- `test-boot`'s serial output now passes through a pipe; behaviour is
  preserved line-for-line, and this is the note recording that it is
  deliberate.
- Injection is unconditionally on for `test-boot` — a check that only
  sometimes receives its stimulus is a check that only sometimes can fail.
- The QMP failure path was broken once on purpose (the `-qmp` argument
  deleted): the run went red on the keystroke check rather than hanging.
- On real hardware there is no QMP and no injector; the same check would go
  red, which is correct — nothing outside presses a key there either. The
  self-test feature never ships in an image (D-28), so this changes nothing
  for a real boot.
