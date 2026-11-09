# D-421 VMware delayed-read verifier follows the kernel format literal

Status: Accepted, 2026-10-17. Implemented with qualification OPEN; amends D-240 and D-372; advances PW-001 and PW-003.

**Context.** A final VMware preflight on `5b66578` found that the kernel's
delayed-read witness had changed under D-372 from
`retained=true interrupts=true` to `retained=true polled=true`, while the
VMware selector still required the old field name. Its host transcript fixture
duplicated the verifier string, so ordinary tests let the two stale consumers
agree with each other. A current successful kernel boot would therefore have
been rejected during human VMware qualification. The complete mutation run on
that source was stopped before its first mutation rather than spending days
qualifying a revision that could not pass the machine gate.

**Decision.** One `SELFTEST_DELAYED_READ_MARKER` owns the verifier and its
canonical transcript fixture. A dedicated host test reads the kernel's unique
delayed-read format literal, renders each of its six success placeholders as
`true`, refuses any unbound field and requires byte-for-byte equality with that
marker. The verifier now requires the producer's current `polled=true` record.
Mutation 2,164 restores the stale `interrupts=true` spelling only in the
verifier constant and must red that source-literal test. This lexical guard
detects the observed accidental drift; it does not prove the literal's Rust
callgraph or replace the fresh runtime VMware transcript. The active registry
therefore contains 2,133 mutations and the next permanent identity is 2,165.

**Proof.** The scoped VMware module passes all 30 tests with host symlink
privilege. The mutation-registry module passes its 20 active tests with one
intentional child fixture ignored. Applying mutation 2,164 makes only
`the_delayed_read_marker_matches_the_unique_kernel_format_literal` fail with the exact
`polled=true` versus `interrupts=true` mismatch; restoring the source makes it
green. Formatting and warning-denying `xtask` Clippy checks pass.

**Qualification boundary.** The repair changes the frozen source, mutation
registry and qualification verifier. Every ordinary and exceptional result
that predates its commit is diagnostic only. PW-001 and PW-003 remain open
until the new clean commit passes the complete 2,133-entry sweep and a fresh
self-test VMware generation, human input transcript and selector qualification.

**Alternatives.** Changing only the stale literal was rejected because the
fixture could drift with it again. Sharing only a verifier-side constant was
insufficient for the same reason. Importing host tooling into the no-std kernel
or weakening the selector to accept either field was rejected: the fresh
runtime record is the authority, and one exact current schema is required.
