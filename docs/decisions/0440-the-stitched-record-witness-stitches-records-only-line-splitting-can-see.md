# D-440 The stitched-record witness stitches records only line splitting can see

Status: Accepted (2026-10-24); strengthens D-421 and D-426; advances PW-003

The complete sweep on `891f22d` reported mutation 110 survived. It replaces
the VMware verifier's one-record-per-line reader with one that splits a line
at every embedded `[info ] ` prefix, so unrelated milestones stitched onto one
physical line would each count as a record. Its witness merged the APIC record
onto the end of the preceding line. D-426 reordered the fixture to the real
boot's order, which put user isolation directly before the APIC. The
isolation validator reads whole lines, so it refused the merged line as a
malformed isolation record before the stitched APIC record could matter, and
the witness stayed green under the mutation for an unrelated reason.

The witness now stitches the ring-three record onto the APIC line. Only the
record reader separates those two messages, so the pristine verifier refuses
the merged transcript and the mutated reader accepts it. In a scratch clone
on `891f22d`, the corrected witness passed, failed at its stitched-record
assertion with mutation 110 applied, and passed again after byte-for-byte
restoration. The other 69 retained mutations of `xtask/src/vmware*` were each
applied there and reddened their named witnesses.

The verifier, its markers and the mutation are unchanged. Current CI, fresh
VMware and the complete sweep remain required on the new candidate.
