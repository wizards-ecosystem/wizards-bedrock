# D-365 VMware qualification binds configuration and preserves runtime bytes

Status: Accepted, 2026-10-07. Amends D-218, D-235 and D-244; advances PW-579.

**Context.** The human boots also expose refusal of the UART's normal CRLF
and of the runtime VMX's postboot digest. Workstation rewrites line endings
and comment encoding and adds assignments. Every original generated
assignment in the observed runtime VMX still matches.

**Decision.** Accept LF and CRLF terminators, refusing bare or embedded CR
and every previously refused control. Preserve raw serial bytes. The
one-boot, one-verdict and ordered hardware witnesses remain mandatory.

The generation receipt continues to bind the exact source-rendered preboot
VMX. Verification recreates it from source, selector and serial generation,
checks runtime assignments against it, and uses the regenerated input digest
when comparing the receipt. Every generated assignment must exist unchanged.
Original key spellings are preserved; duplicate keys, including case-colliding
keys, unknown additions and alternate quoting/escape forms refuse. Only
individually constrained, observed runtime
assignments are allowed. Device placements, graphics and USB defaults get
explicit constraints rather than unrestricted metadata permission.
The passing record separately hashes the exact captured runtime VMX and
revalidates it before publication and offline verification. Neither receipts
nor serial output are rewritten to pass.

**Proof required.** Fixtures accept CRLF and the reviewed runtime additions;
reject embedded/bare CR, missing or changed generated assignments, duplicate
or case-colliding keys, unknown additions and invalid runtime values; and
preserve exact capture while refusing mixed generations and later changes.
Each new acceptance path and guard needs a retained red mutation, exact
restoration and CI. Fresh human VMware records remain required.

**Alternatives.** Ignoring VMX loses the machine contract. Updating the
receipt to the postboot digest rewrites provenance. Removing arbitrary
additions before hashing can hide CPU or device overrides. Stripping every
CR turns terminal-control sequences into apparently canonical evidence.

**Still unresolved.** Both guests report architectural capabilities `0x44`,
without RDCL_NO, while Workstation logs guest MSR `0x2f`; the host uses the
Hyper-V ULM monitor. PW-580 investigates the discrepancy. Its cause is not
established, and D-250's affirmative isolation requirement stays unchanged.

**Host evidence.** On 2026-10-07 all 38 VMware-focused host checks passed.
Mutations 1,606 through 1,622 each selected one exact witness, returned exit
101 with that witness marked FAILED and restored source byte for byte.
They exercise CRLF admission, terminal-active CR refusal, generated-value
binding, unknown and duplicate keys, bounded identity fields, assignment
grammar, exact capture and source-generated input identity. Full CI is
required before committing this slice; passing human records remain owed.
Mutation 1,624 also failed changed-key-spelling admission. Tightening that
rule initially masked the duplicate-key witness with the unknown-key guard;
the repaired fixture independently rejects identical duplicate assignments
at the parser, and mutation 1,610 then failed that exact witness again.
