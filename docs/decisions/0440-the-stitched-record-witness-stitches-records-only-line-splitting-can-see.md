# D-440 The stitched-record witness stitches records only line splitting can see

Status: Accepted, 2026-10-24. Strengthens D-421 and D-426; advances PW-003.

**Decision.** The witness for mutation 110, which splits a VMware transcript
line at every embedded `[info ] ` prefix, stitches the ring-three record onto
the APIC line. Only the record reader separates those two messages, so the
pristine verifier refuses the merged transcript and the mutated reader accepts
it. In a scratch clone on `891f22d` the corrected witness failed at its
stitched-record assertion under mutation 110 and passed after restoration, and
the other 69 retained mutations of `xtask/src/vmware*` each reddened their
named witnesses. The verifier and the mutation are unchanged. Current CI,
fresh VMware and the complete sweep remain required on the new candidate.

**Rationale.** The complete sweep on `891f22d` reported mutation 110 survived:
after D-426 moved user isolation directly before the APIC, the witness's
APIC-onto-previous-line merge was refused by the whole-line isolation validator
before the stitched record mattered, so it stayed green for an unrelated
reason.
