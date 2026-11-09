# D-428 The ISA DMA proof checks the adapter operations as well as their caller

Status: Accepted (2026-10-23), qualification OPEN; strengthens D-426 and PW-597

**Context.** D-426's pure entry-fence tests correctly required an ISA quiesce
operation, but its platform contract checked only that the operation called
`IsaDma::quiesce`. Neither that contract nor a machine self-test checked the
method's master-clear, all-mask, status-read, or later remask instructions.
Deleting the reset and mask writes while retaining the call and quiet status
reads therefore left the existing contract satisfied. A firmware handoff that
was already quiet could also make a live VMware boot pass without testing the
missing work. A call to a function is not evidence of the hardware actions that
function must perform.

**Decision.** A dedicated host source contract checks the parsed executable
statements at the ISA adapter. The entry sequence must issue byte-wide master
clears at DMA1 offset `0x0d` and DMA2 offset `0x1a`, then run the shared remask.
The remask must write `0x0f` to DMA1 offset `0x0f` and DMA2 offset `0x1e` before
reading status at offsets `0x08` and `0x10`. Each controller's byte occupies its
own half of the combined word, and the pending-request mask is exactly
`0xf0f0`. A later quiet check must hold the permanent controller owner, refuse
an absent owner, issue a fresh remask, and test that exact mask. The entry path
must move both capabilities into permanent storage before returning either
success or refusal. The paired claim must reserve both complete fixed windows
in one transaction.

Permanent mutations 2,307 through 2,321 independently remove each controller's clear and mask,
misdirect each status read, alias or shift the returned status bytes, drop a
pending-request bit, bypass the later remask or missing-owner refusal, discard
ownership on success or refusal, and move the paired claim to the wrong base.
On 2026-10-23 the pristine adapter contract passed, all fifteen mutations failed
its exact named witness, and the restored pristine contract passed again. Each
of the three transiently changed source files was restored byte-for-byte and
checked by SHA-256. Full qualification remains the current ordinary gate,
complete curated mutation sweep, and fresh source-bound VMware records.

**Alternatives.** Checking only for function-name text was rejected because it
was the gap this review found. Requiring mask-register readback was rejected
because VMware does not expose one. A new generic runtime I/O interpreter was
unnecessary for this small fixed platform adapter; the parsed ordered contract
checks the actual operations while leaving the production instruction path
unchanged.
