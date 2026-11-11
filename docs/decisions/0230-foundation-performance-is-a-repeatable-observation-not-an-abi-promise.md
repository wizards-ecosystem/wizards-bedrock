# D-230 Foundation performance is a repeatable observation, not an ABI promise

Status: Accepted, 2026-08-29. Closes PW-511.

**Amended by D-384.** The bands below are the original reviewed values; D-384
replaces the footprint, free-frame, live-allocation and upper timing values and
keeps the rest.

**Decision.** `cargo xtask baseline` is a bounded qualification task over the
pinned QEMU machine, not a CI stage and not a benchmark of hardware. It builds
the self-test image once per debug and release profile and boots each image
three times. Every boot must retain the ordinary complete self-test verdict.
The task reports every sample, the observed range and a versioned, separately
reviewed threshold band for each profile; those bands are regression alarms,
not ABI or throughput guarantees.

Host measurement owns facts the guest cannot improve by reporting them:
kernel ELF bytes, fixed disk-image bytes and the syntactic unsafe-line count.
The self-test kernel emits one strict `baseline v1` record after all checks. It
records calibrated time from clock origin to the start of self-test, free
frames, all three heap counters, committed and occupied handle-arena slots,
the TSC rate, one ring-3 `int 0x80` Echo round trip and one maximum-buffered
4 KiB NVMe read. The round trip brackets the actual user instruction on both
sides with fenced timestamp reads and leaves its delta in an otherwise unused
submission field after the kernel has consumed the request, so it measures the
boundary and its minimal real queue service, the path a Wizzy runtime will
call, rather than a kernel function timed from inside itself. Storage brackets
a real polled DMA command and converts ticks with the same calibrated rate the
kernel publishes. Baseline state lives outside the self-test coordinator's
frame, which had no stack headroom to retain another resource snapshot.

The two dynamic witnesses are also ordinary machine checks: zero ticks, a
faulting child, a missing completion or a read which did not fill the whole
buffer is a correctness failure before it is a performance sample. The host
parser refuses duplicate records, missing or reordered fields, unknown
versions, arithmetic overflow and a baseline attached to anything but a clean
verdict. Pure tick conversion and band/variance evaluation stay in `xtask`,
where host tests cover their boundaries.

The reviewed release bands are 6.4 to 7.1 MB ELF, 3.5 to 9 ms boot, 114,800
to 115,000 free frames, 16 to 24 slab / 990 to 1,010 large / exactly 4 live
heap allocations, empty handles, 10 to 100 microseconds at the ring boundary
and 10 to 100 MiB/s storage. Debug uses 8.2 to 9.1 MB, 20 to 45 ms, 114,150
to 114,400 frames, 14 to 24 / 990 to 1,010 / exactly 4 heap state, empty
handles, 50 to 400 microseconds and 3 to 20 MiB/s. Both require a 3 to 4 GHz
calibrated counter, the exact 64 MiB image, exact 4 KiB read and the existing
exact unsafe count. No threshold is a compatibility promise, and a threshold
change is a reviewed measurement update rather than an ABI decision.

**Rationale.** The foundation had correctness gates but nothing caught a
correctness-preserving change which doubled the kernel image, retained a new
class of memory or slowed a boundary path by orders of magnitude, and
`010-research-notes-3.md` records that QEMU/TCG time is not bare-metal
performance evidence. Treating one emulator duration as a product promise would
manufacture precision, while recording nothing leaves the cheapest large
regressions invisible.

**Alternatives.** Host wall-clock time was rejected because scheduler load can
move it without changing one guest instruction. `-icount` as a universal
answer was rejected because it changes the virtual timer model whose real
calibration, sleeps and I/O this image is meant to exercise; instruction count
can become an additional baseline later without replacing these paths. A
single run was rejected because it cannot state variance. Putting six more
boots in CI was rejected because the bands are descriptive qualification and
the ordinary debug/release boots already gate correctness. Timing only kernel
functions was rejected because it bypasses the ring-3 and device boundaries
whose regression matters to the future Wizzy caller.

**Consequences.** Mutations 266 through 271 keep the two production checks and
the strict-record, two-sided-band, documented-command and emitted-selector host
tests red. No unsafe operation or wire ABI changed.
