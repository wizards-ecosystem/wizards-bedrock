# D-195 NVMe reads cross the doorbell only as validated transfers

Status: Accepted, 2026-08-27. Amends D-169; closes PW-203.

**Context.** `Controller::read_blocks` multiplied an unchecked block count by
the namespace's block size, built `CDW12` as `blocks - 1`, and submitted it.
Zero underflowed in debug and encoded the maximum transfer in release. Counts
above 65,536 were truncated by the command's sixteen-bit zero-based NLB field.
Neither `lba + blocks` nor the reported namespace extent participated, and the
driver proved only that the DMA run had an address, not that it held every byte
the controller was told it could write.

These are arithmetic obligations, so the controller was the wrong place to
own them. They also fail before publication: treating any as a command error
would ring a doorbell with a malformed extent and make the refusal itself a DMA
memory-safety hazard.

**Decision.** `libs/nvme::ReadTransfer` is the only value the read submission
path accepts. Its constructor requires a non-empty count no greater than 65,536,
a non-zero block size, an end which adds without overflow and does not exceed
the namespace, a byte length which multiplies without overflow, and a DMA byte
capacity at least that large. It retains the LBA, exact byte length and the
already-proved zero-based NLB encoding behind read-only accessors.

`DmaSegment` exposes its owned byte length as an `Option`, alongside its
optional physical address: retained memory is neither addressable nor sizeable.
The controller converts raw caller arguments into `ReadTransfer` before taking
a command identifier, touching a queue entry, changing DMA ownership or writing
a doorbell. The private submission function accepts only that validated value.

The self-test build can intercept that private submission seam. Invalid raw
requests run through the production validator; if a defect admits one, the
interceptor counts it and returns before queue or MMIO mutation. That permits a
real-machine proof that zero, oversized-NLB, namespace-end/overflow and short
DMA requests stop before the doorbell without asking hardware to execute a
known-invalid command.

**Rationale.** A checked length alone is insufficient: a representable byte
count can still describe blocks outside the namespace or a count the command
field truncates. Checking after PRP construction is too late to make the command
shape authoritative, and PRP construction cannot know the allocation's end.
A validated value makes each downstream field a projection of one proof rather
than a second transcription of raw arguments.

**Alternatives.** Inline conditionals in `kernel::nvme` were rejected because
pure bounds would lose host tests and the submission path would still accept raw
values. Clamping was rejected because a successful short read silently changes
the requested operation. Relying on current one-block callers was the defect:
the driver API, not today's call sites, owns the DMA boundary. Submitting the
invalid commands to QEMU for a negative check was rejected because an oversized
DMA write can corrupt the test kernel before it reports a verdict.

**Consequences.** Host tests exhaust the entire encodable NLB count boundary
and cover empty, namespace-last, wrapping, zero-block-size, length-overflow and
one-byte-short DMA cases. The machine check uses the actual controller and DMA
owner while intercepting the final submission seam, requires every malformed
request to be refused, and requires command and interception counts to remain
unchanged. Empty, NLB-width, namespace-wrap, short-buffer and driver-integration
mutations each reddened their exact check; the last produced only `invalid nvme
reads stop before submission` with `passed=199 failed=1 expected=200` and could
not ring real hardware while broken. The release image passed all 200 checks;
the mutation register now contains 107 entries. No unsafe operation was added.
