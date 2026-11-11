# D-38 The harness can press a key

Status: Accepted, 2026-08-20. Extends D-28.

**Decision.** `cargo xtask test-boot` injects a keystroke through a
hand-rolled QMP client. It binds a loopback `TcpListener` on an ephemeral
port before spawning QEMU and passes `-qmp tcp:{addr}`, so QEMU dials out:
no fixed port to collide, no bind-then-connect race, no retry loop. A
forwarding thread pipes serial, still passing every line to the console and
flushing a dying kernel's unterminated last line. It watches for the line
the kernel prints just before its bounded wait,
`selftest: awaiting injected keystroke`, a two-file contract kept in step by
comment like the exit codes. The harness then sends one `input-send-event`
command with an explicit press and release of `A`. The kernel's check
requires the make, then the break, to decode in order.

**Rationale.** An input device's defining behaviour starts outside the
machine. `sendkey` releases the key on QEMU's own schedule, and a QMP crate
is a dependency for three fixed exchanges (D-25's precedent). Keys wait for
the line because the self-test's own controller commands must never
interleave with injected bytes. The verdict stays the kernel's: a QMP
failure retires the injector, and the kernel's doubly bounded wait (a tick
budget, plus an iteration cap so a dead timer cannot turn red into a hang)
then fails the check. Replies are classified by token, not parsed as JSON.

**Consequences.**

- Injection is always on for `test-boot`: a check that only sometimes gets
  its stimulus only sometimes can fail.
- Real hardware has no QMP or injector, so the check would go red there,
  correctly. The self-test feature never ships in an image (D-28), so a real
  boot is unchanged.
