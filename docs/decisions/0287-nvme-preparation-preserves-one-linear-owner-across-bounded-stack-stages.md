# D-287 NVMe preparation preserves one linear owner across bounded stack stages

Status: Accepted (2026-09-05), amends D-217, D-245 and D-278; advances PW-541

**Context.** A local guarded debug boot reproduced the storage-path double fault
that D-245 had not reproduced. Local frame inspection reported a 16,200-byte
`nvme::prepare` frame beneath `try_controller`'s 16,184-byte frame on the 32 KiB
boot stack. The individual frame limit stayed green while the live nested path
exhausted the stack.

**Decision.** Preparation is a sequence of ownership-preserving stages:
capability read, disable, admin allocation, register programming and final
enable. `AdminPreparation` carries the one controller and DMA owner forward;
each stage returns a bounded explicit outcome whose refusal either transfers
that owner to the next stage or retains it in the existing quarantine. The
same local inspection reported a largest conservative linked path of 22,980
bytes, leaving 9,788 bytes on the guarded stack.

**Alternatives and evidence.** Enlarging the stack would hide rather than bound
the path. Boxing the state would add allocation and failure at the recovery
boundary. Per-function `.debug_frame` limits cannot prove nested use. The
causal red and frame report informed the split but are not durable phase-close
receipts. After the split the guarded boot reached and passed every NVMe
witness. PW-541 requires a reproducible retained frame measurement and current
complete debug, release and integrated runs.
