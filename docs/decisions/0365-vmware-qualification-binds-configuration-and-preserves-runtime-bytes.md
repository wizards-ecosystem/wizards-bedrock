# D-365 VMware qualification binds configuration and preserves runtime bytes

Status: Accepted, 2026-10-07. Amends D-218, D-235 and D-244; advances PW-579.

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

**Rationale.** The human boots exposed refusal of the UART's normal CRLF and
of the runtime VMX's postboot digest. Workstation rewrites line endings and
comment encoding and adds assignments, while every original generated
assignment in the observed runtime VMX still matched.

**Alternatives.** Ignoring VMX loses the machine contract. Updating the
receipt to the postboot digest rewrites provenance. Removing arbitrary
additions before hashing can hide CPU or device overrides. Stripping every
CR turns terminal-control sequences into apparently canonical evidence.

**Consequences.** Both guests reported architectural capabilities `0x44`,
without RDCL_NO, while Workstation logged guest MSR `0x2f`; the host used the
Hyper-V ULM monitor. PW-580 investigated the discrepancy. Its cause was not
established, and D-250's affirmative isolation requirement stays unchanged.

Retained mutations 1,606 through 1,622 and 1,624 each fail one exact witness,
covering CRLF admission, terminal-active CR refusal, generated-value binding,
unknown and duplicate keys, bounded identity fields, assignment grammar,
exact capture, source-generated input identity and changed key spelling.
Identical duplicate assignments are rejected at the parser independently of
the unknown-key guard, which mutation 1,610 witnesses. Fresh human VMware
records remained owed to PW-579.
