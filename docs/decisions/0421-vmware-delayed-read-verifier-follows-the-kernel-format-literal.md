# D-421 VMware delayed-read verifier follows the kernel format literal

Status: Accepted, 2026-10-17. Implemented with qualification OPEN; amends D-240 and D-372; advances PW-001 and PW-003.

**Decision.** One `SELFTEST_DELAYED_READ_MARKER` owns the verifier and its
canonical transcript fixture. A dedicated host test reads the kernel's unique
delayed-read format literal, renders each of its six success placeholders as
`true`, refuses any unbound field and requires byte-for-byte equality with that
marker. The verifier now requires the producer's current `polled=true` record.
Mutation 2,164 restores the stale `interrupts=true` spelling only in the
verifier constant and must red that source-literal test. This lexical guard
detects the observed accidental drift; it does not prove the literal's Rust
callgraph or replace the fresh runtime VMware transcript.

**Qualification boundary.** Every ordinary and exceptional result that
predates the repair is diagnostic only. PW-001 and PW-003 remain open until the
new clean commit passes the complete sweep and a fresh self-test VMware
generation, human input transcript and selector qualification.

**Rationale.** D-372 changed the kernel's delayed-read witness from
`retained=true interrupts=true` to `retained=true polled=true`, but the VMware
selector and its host fixture duplicated the old string and agreed with each
other, so a successful boot would have been refused during human
qualification. Changing only the literal or sharing only a verifier-side
constant lets the fixture drift again. Importing host tooling into the no-std
kernel or accepting either field would displace the fresh runtime record as
authority; one exact current schema is required.
