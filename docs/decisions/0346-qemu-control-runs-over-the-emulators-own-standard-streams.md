# D-346 QEMU control runs over the emulator's own standard streams

Status: Accepted, 2026-09-28. Amends D-38, D-227 and D-231; closes PW-557.

**Context.** D-227 made the harness listen on loopback before QEMU dialled its
QMP and human-monitor connections. That removed a port race but left each
listener open to whichever local process connected first. Three retained
Daybreak findings follow from it. A process that connected first became the
harness's QMP and monitor peer, so it could swallow or fake keystrokes and
forge the shipped boot's keyboard-interrupt counts, which enter release
evidence. The monitor reader collected text until a prompt with no byte
limit, the QMP reader read lines of any length, and a peer that sent a byte
within every per-read timeout could hold a boot open indefinitely. The
injector checked cancellation only between commands.

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

**Evidence.** On 2026-09-28 QEMU 11.0.50 greeted, negotiated and answered
`human-monitor-command` over piped stdio, and a guest's serial output reached
a followed file within 1.7 s. A release self-test boot then passed 262/262
with five keystrokes injected over the private channel, the shipped check saw
keyboard IRQ 1 rise from 24 to 30 through `info irq`, and the debugger proof
stopped at `kmain` at an endpoint discovered over QMP. Tests against a fake
emulator on in-process pipes refused a reply naming another command, a
refusal that also carried a return, a message over 1 MiB (while the fake's
next 4 MiB were still drained), a silent peer at its deadline and a cancelled
exchange, and quoted monitor text exactly. A serial follower read every
appended byte and ended only after close, and a run file skipped 256 occupied
names. Mutations 1,510 through 1,520 each weaken one of these rules and each
failed its exact witness before byte-for-byte restoration. Mutations 516 and
797 moved to the run-file reservation that the firmware store and the serial
file now share, and failed their witnesses again. Mutations 514, 515 and 517,
which guarded the removed listeners, are retired.

**Alternatives.** A per-run secret on the loopback connection was rejected
because QMP has no pre-shared-key handshake, and TLS client certificates would
add a key store for one local pipe. Checking the connecting process through
the system's TCP owner table needs Windows APIs the harness cannot call
without `unsafe`. A Windows named pipe was rejected because QEMU, as its
server, would still accept whichever local client connected first. The GDB
stub stays on loopback because GDB speaks TCP; D-231 already limits it to a
paused private machine.
