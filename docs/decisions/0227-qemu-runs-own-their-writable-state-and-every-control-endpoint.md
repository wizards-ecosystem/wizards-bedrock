# D-227 QEMU runs own their writable state and every control endpoint

Status: Accepted (2026-08-29), closes PW-512

**Context.** QMP already avoided a bind/connect race by making `xtask` listen
before QEMU dialled it, but the human monitor did not. Its port was found by
binding port zero, reading the result, dropping the socket and asking QEMU to
bind the same number later. Another local process could take that port in the
gap. Every run also copied the firmware variable template to the same
`target/efi-vars.fd`; two otherwise independent runners could therefore write
one firmware store. The first two-run experiment exposed the same defect in
the boot artifact itself: both builders truncated `target/wizard-os.img`, and
one failed while reading back a short `wizard-kernel.elf`. A first construction
of the shared NVMe corpus had no cross-process exclusion either. Finally,
every disk path was interpolated into QEMU's comma-separated `-drive`
key/value syntax without escaping commas. `Command` preserved spaces as one
argument, but that did not make the argument's inner grammar safe.

**Decision.** Each boot reserves both QMP and human-monitor loopback listeners
before spawning QEMU. QEMU is the client for both; the exact human-monitor
suffix is `server=off`, because a real boot proved that QEMU rejects `wait` in
client mode. The harness accepts and keeps one monitor stream, consumes its
initial prompt once and uses the same bounded connection for every later
query. No control port is ever probed, dropped and rebound.

Each run creates a unique variable-store file with `create_new` beneath the
fixed `target/qemu runs,private/` directory, copies the pristine firmware into
that already-owned file and removes it on drop. The process ID plus a
monotonic in-process sequence makes names diagnostic; `create_new`, not the
name, establishes exclusivity. A QEMU consumer likewise reserves its own boot
image with `create_new` in that directory and removes it on drop; the explicit
`cargo xtask image` artifact remains the canonical `target/wizard-os.img`.
The NVMe corpus moves beneath `target/qemu storage,private/`, so every gate
boot exercises a real path containing both a space and comma. An OS advisory
lock serializes its read/repair/write transaction with a five-second bound.

All file-valued `-drive` arguments go through one encoder which doubles commas
for QEMU's key/value grammar. Spaces remain ordinary bytes in the same
`OsString`; no shell is involved. Device-option commas are still syntax and
are not escaped. Shared boot and NVMe bytes use QEMU's private snapshot mode:
the emulated device may write, but those writes cannot alter another run's
base. The QMP injector also receives the run deadline and a cancellation flag
and is joined on every exit path. A harness-requested stop waits for QEMU to be
reaped before private-file guards drop, so Windows has released the handles
when cleanup runs.

**Rationale.** Holding a kernel socket is the only local proof that its port
is reserved. Retrying a guessed port merely narrows the race. Writable UEFI
state and a boot under construction belong to one virtual machine, not shared
build output. A read-only NVMe backend was rejected by the real QEMU
controller, so a temporary snapshot is the isolation primitive it can consume.
Exercising the hostile directory names in every real boot proves the path
encoder against the same QEMU parser and bytes the gate relies on, while a pure
argument test identifies the exact doubling rule when a boot fails before
storage comes up.

**Alternatives.** Fixed ports were rejected because they make concurrent runs
mutually exclusive. Unix sockets were rejected as the only route because the
host gate is Windows. Letting QEMU remain the monitor server with a retry loop
was rejected because it still cannot reserve the chosen port across process
launch. Quoting the whole `-drive` value was rejected because `Command` does
not invoke a shell and quotes would become literal QEMU input; the comma is
inside QEMU's grammar and must be escaped there.

**Consequences.** Concurrent runners have disjoint control sockets, firmware
stores, boot images and guest-write overlays; shared NVMe construction is
serialized and a completed corpus is reused only after its complete contents
match. Two simultaneous shipped boot checks both built, booted, advanced their
keyboard IRQ count from 24 to 30 and settled; the failed precursor is what
found the shared-image race. Release self-test QEMU passed 213/213 through the
real comma/space paths, and the shipped check exercised repeated commands on
one accepted monitor stream. Reaping before teardown left the private-run
directory empty. Eight deliberately broken host witnesses went red and persist
as mutations 251 through 258. No machine check was added.
