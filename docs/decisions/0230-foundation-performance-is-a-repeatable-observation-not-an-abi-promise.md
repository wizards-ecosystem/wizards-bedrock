# D-230 Foundation performance is a repeatable observation, not an ABI promise

Status: Accepted (2026-08-29), closes PW-511

**Amended by D-384.** The original observed bands remain historical evidence;
the current reviewed footprint and retained-state bands are recorded below.

**Context.** The foundation had correctness gates but no stable answer to a
correctness-preserving change which doubled the kernel image, consumed a new
class of retained memory or made a boundary path orders of magnitude slower.
`010-research-notes-3.md` also records the essential limit: QEMU/TCG time is
not bare-metal performance evidence. Treating one emulator duration as a
product promise would manufacture precision, while recording nothing leaves
the cheapest large regressions invisible.

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
submission field after the kernel has consumed the request. It is therefore a
measurement of the boundary and its minimal real queue service, the path a
Wizzy runtime will call, rather than a kernel function timed from inside
itself. Storage brackets a real polled DMA command and converts ticks with the
same calibrated rate the kernel publishes.

The two dynamic witnesses are also ordinary machine checks: zero ticks, a
faulting child, a missing completion or a read which did not fill the whole
buffer is a correctness failure before it is a performance sample. The host
parser refuses duplicate records, missing or reordered fields, unknown
versions, arithmetic overflow and a baseline attached to anything but a clean
verdict. Pure tick conversion and band/variance evaluation stay in `xtask`,
where host tests cover their boundaries. Each new check gets its deliberate
red witness and persistent mutation.

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

**Consequences.** The tightened task passed all six 215-check boots. Release
observed 5,109,630 to 5,378,776 ns to self-test, 114,885 free frames, heap
state 20/1,002/4, empty handle state, 24,552 to 26,982 ns at the trap boundary
and 26,869,503 to 33,474,843 bytes/s from storage. Debug observed 26,835,732 to
29,484,274 ns, 114,265 to 114,266 frames, heap 20/1,002/4, empty handles,
105,937 to 201,668 ns and 7,479,097 to 8,701,422 bytes/s. Both profiles read
exactly 4,096 bytes. Their kernel ELFs were 6,725,512 and 8,648,304 bytes;
both images remained exactly 67,108,864 bytes; `wost` remained 380 unsafe
lines in 170 blocks.

The reviewed release bands are 6.4 to 7.1 MB ELF, 3.5 to 9 ms boot, 114,800
to 115,000 free frames, 16 to 24 slab / 990 to 1,010 large / exactly 4 live
heap allocations, empty handles, 10 to 100 microseconds at the ring boundary
and 10 to 100 MiB/s storage. Debug uses 8.2 to 9.1 MB, 20 to 45 ms, 114,150
to 114,400 frames, 14 to 24 / 990 to 1,010 / exactly 4 heap state, empty
handles, 50 to 400 microseconds and 3 to 20 MiB/s. Both require a 3 to 4 GHz
calibrated counter, the exact 64 MiB image, exact 4 KiB read and the existing
exact unsafe count.

The exploratory first debug repetition also found the self-test coordinator
had no headroom to retain another resource snapshot: `Option<AddressSpace>`
needed an 872-byte debug frame and crossed the guarded boot stack. Baseline
state now lives outside that coordinator frame; one direct debug boot and both
subsequent three-run qualifications passed the formerly faulting path. The two
production breaks each reddened only their named machine check at 214/215;
strict-record, two-sided-band, documented-command and emitted-selector breaks
each reddened only their exact host test. Mutations 266 through 271 retain
those defects. No unsafe operation or wire ABI changed. No threshold is a
compatibility promise, and a threshold change is a reviewed measurement update
rather than an ABI decision.
