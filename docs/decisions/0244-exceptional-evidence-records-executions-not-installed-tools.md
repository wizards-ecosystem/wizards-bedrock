# D-244 Exceptional evidence records executions, not installed tools

Status: Accepted, 2026-08-30. Amends D-216, D-218 and D-235; closes PW-007 and PW-008; advances PW-009.

**Decision — exceptional records.** Mutation, TLC and each VMware selector own
one sorted, checksummed qualification record under ignored target output. An
attempt removes the older success before its first fallible preflight and can
commit `result=pass` only after its terminal operation. Every record binds the
clean Git commit and tree. Mutation additionally binds the registry, source
and index digests and the durable complete report. TLC binds the resolved Java
bytes and version, the TLC jar and the complete tracked model tree. VMware
binds the resolved `vmrun`, generation receipt, both disks, both descriptors,
VMX, serial transcript and execution observer. The release evidence format is
v3 and carries every one of those support files and digests; offline
verification reconstructs their inner schemas instead of trusting only the
outer manifest.

Every record rechecks clean source immediately before rename. The mutation
runner also re-digests its supposedly immutable copied source at completion,
before publishing `last-run` or a passing report. A changed copy can therefore
invalidate work, but cannot authorize pristine source.

Mutation completion writes a canonical, checksummed, immutable sequenced
report whose caught, survived and inconclusive totals determine its explicit
verdict. Older reports remain available, but only the greatest sequence for
the current clean commit, registry, source and index can authorize a pass. The
returned current-pass value owns the mutation sweep lock until its consumer
finishes; mutation qualification and the release-evidence snapshot retain that
value through publication, so a newer failed sweep cannot race an older pass
into the bundle.

TLC gets a hard 30-second process deadline. Its observer requires empty
diagnostics, exactly one terminal verdict before exactly one final completion
record and an exit status consistent with that verdict. A clean verdict has no
counterexample trace; an invariant violation has exactly one later behavior
header and at least one state before completion. Wrong invariants, mixed
verdicts, truncated traces, unrelated `Error:` records, Java throwables, stack
records and fatal worker text are refusals rather than mutation catches.

**Decision — one generated VMware machine.** D-235's selectors now own
`vm/self-test/` and `vm/shipped/`. Generation removes only that exact selector
directory after real-directory and canonical-parent checks, then constructs an
exact initial inventory: five authoritative machine artifacts, one empty
`serial-<generation>.log`, and the generation receipt (the incomplete receipt
occupies that seventh name until the atomic rename). NVRAM, locks, logs,
scoreboards and suspend state are absent at generation. VMware may create them
after boot, but they are explicitly runtime extras and carry no authority.
The five artifact names, receipt and transcript must remain regular files;
substituting a link, directory or another canonical parent is a refusal.

The generation ID is a new per-invocation local identifier derived from the
host clock, process and an in-process sequence. Its purpose is to give VMware a
new serial destination and stop accidental reuse of `serial.log`; it is not a
secret, nonce supplied to the guest, or cryptographic attestation. The receipt
binds the empty reset, exact source, selector and initial artifact digests.
Qualification requires the transcript at the receipt's generated name to have
become nonempty and rechecks every authoritative digest before and after it is
copied.

**Decision — observe execution.** `VmwareAttempt` retains the exact resolved
tool. Qualification probes those bytes, executes that path with exactly
`-T ws list`, requires successful exit and the canonical VMware count header,
and accepts exactly one absolute VMX path. Canonicalizing that path must produce
the generated selector's regular VMX. The tool is probed again around the
observation and before commit. The normalized command output is stored as
`vmrun-list.txt`; its length and digest are in the checksummed inner record and
the outer evidence manifest. Availability text can no longer stand in for
execution, another running VM cannot share the observation, and a foreign VMX
cannot donate it.

All destructive evidence and qualification paths are revalidated at the point
of use. A real root and every real parent are required before recursive reset,
copy or record commit, canonical children must remain under the exact configured
root, and new staging/support files use exclusive creation. Qualification and
generation records require their one canonical sorted LF encoding and
checksum; the outer manifest requires canonical encoding and verifies every
recorded digest. These checks prevent a linked parent with identical bytes
from substituting provenance, not merely a different digest from being
noticed.

**Trust boundary.** This is strong local qualification, not remote
attestation. It proves that the recorded `vmrun` returned one running canonical
VMX at the observation, that the selected generated files retained their
recorded bytes, and that the accepted transcript occupied that generation's
serial path. It does not prove that `vmrun` started the VM, that those serial
bytes were emitted by that process, or that a privileged operator, host
filesystem, hypervisor or `vmrun` did not fabricate them. In particular, an
operator can copy an older valid transcript into the fresh path. There is no
guest-consumed per-generation challenge with which to reject that act, and the
checksums are integrity checks rather than signatures. PW-531 stops stronger
process-to-transcript binding until a concrete guest-readable challenge channel
or platform attestation mechanism exists.

**Rationale.** The rejected phase handoff treated recording a tool's identity
as recording its work. `last-run` had no overall verdict and let an older pass
authorize closure after a newer sweep failed; the model observer accepted any
nonzero exit containing the invariant text and could wait forever;
`begin_vmware` discarded the resolved `vmrun` and `qualify` accepted any
transcript already on disk. D-235 left every other file in each machine
directory, a copied mutation snapshot was verified only when opened, evidence
parsing accepted reordered fields and a missing terminal LF, and a broken
staging symlink could survive invalidation. These are stale-success,
substitution and path-authority defects, not missing explanatory comments.

**Alternatives.** Keeping a fixed serial filename plus truncation was rejected
because it gives accidental replay one stable destination. Treating every
post-boot VMware file as evidence was rejected because their formats and
lifecycle belong to Workstation; authority rests in the generated inputs and
explicit observer. Parsing any positive `vmrun` count was rejected because it
allows an unrelated running machine to donate execution. Inventing a
cryptographic claim from host-generated metadata was rejected because the
guest never consumes or returns that metadata. Deleting the whole `vm/` root
was rejected because one selector must never destroy its sibling.

**Evidence and consequences.** Host fixtures complete and consume a clean
isolated mutation sweep, retain an older pass beside a newer failure, reject
failed and stale reports and block a concurrent replacement until the current
pass is consumed. TLC fixtures distinguish clean success and the exact named
invariant from wrong status, wrong invariant, mixed verdict, truncation,
unrelated error, diagnostic output, Java throwable, fatal output and a
missing or contradictory behavior trace; a parked child proves the deadline.
Other host fixtures reject zero, noncanonical and multiple `vmrun` counts,
missing and extra paths, relative and foreign VMXs, linked selector and
qualification parents, stale NVRAM, authoritative-name substitution, shared
serial names, linked evidence roots, broken staging links, reordered manifests
and a changed mutation snapshot. The command seam executes exact pre-probe,
`-T ws list` and post-probe order, and production requires the real observer
before result and commit. Each guard was removed or weakened once and its exact
fixture went red; the restored defects persist in the mutation registry. PW-007 and PW-008 are closed as observer
implementations, while the real current-registry mutation and TLC executions
remain PW-001 and PW-002. No real exceptional close is claimed here: PW-009 is
`REQUIRED` until one clean final commit completes mutation, TLC, both human
VMware rounds, both execution qualifications, evidence creation and
independent offline replay.
