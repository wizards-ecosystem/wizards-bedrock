# D-346 QEMU control runs over the emulator's own standard streams

Status: Accepted, 2026-09-28. Amends D-38, D-227 and D-231; closes PW-557.

**Decision.** Every boot that needs control speaks QMP over QEMU's standard
input and output, which only the harness that spawned it holds; `-qmp stdio`
is the only control flag, and no boot binds a control listener. Monitor queries
(`info irq`, `info registers`, `x/8i`, `gdbserver` and `info chardev`) go
through QMP's `human-monitor-command`. The guest's first serial port moves to
a private OS transport consumed only by the harness: a fresh, unguessable
named pipe on Windows and a socket beneath the private run directory on Unix.
Both are bounded and backpressured rather than disk-backed (D-367). The
degraded boot, which uses no control channel, keeps serial output on stdio.

One pump thread reads QEMU's output for the whole run in messages of at most
1 MiB, drops events, and passes the greeting and replies to the caller
through a queue of four. After a protocol failure it keeps draining, so QEMU
never blocks on a full pipe. Every command carries a fresh `id`, and only the
reply naming that `id` can answer it; a message carrying both an error and a
return reads as the error. Each exchange has a deadline and a cancellation
flag, both checked every 20 ms.

**Rationale.** D-227's loopback listeners removed a port race but stayed open
to whichever local process connected first, which could then swallow or fake
keystrokes and forge the shipped boot's keyboard-interrupt counts that enter
release evidence. The monitor and QMP readers had no byte limit, a peer
sending a byte within every per-read timeout could hold a boot open
indefinitely, and the injector checked cancellation only between commands.
Mutations 1,510 through 1,520 retain one break of each rule; mutations 516 and
797 moved to the run-file reservation that the firmware store and the serial
file now share, and mutations 514, 515 and 517, which guarded the removed
listeners, are retired.

**Alternatives.** A per-run secret on the loopback connection was rejected
because QMP has no pre-shared-key handshake, and TLS client certificates would
add a key store for one local pipe. Checking the connecting process through
the system's TCP owner table needs Windows APIs the harness cannot call
without `unsafe`. A Windows named pipe was rejected because QEMU, as its
server, would still accept whichever local client connected first. The GDB
stub stays on loopback because GDB speaks TCP; D-231 already limits it to a
paused private machine.
