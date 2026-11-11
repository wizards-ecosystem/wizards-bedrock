# D-298 Git pathname bytes precede filesystem path semantics

Status: Accepted, 2026-09-10. Amends D-208; closes PW-030.

**Decision.** The tracked inventory is checked for strict byte order and
uniqueness while it is still the NUL-delimited output Git supplied. Each entry
is then required to be UTF-8 and a confined relative path before conversion to
`PathBuf`. Filesystem paths remain the authority for later containment and
file operations; they no longer redefine the ordering of Git's source
identity.

Sorting after conversion was rejected because it would hide producer disorder
and collapse the distinction between validation and normalization. Removing
the order check was rejected because a duplicate or reordered identity could
then reach digesting and workspace publication.

**Rationale.** The first current clean `cargo xtask mutate` stopped before its
baselines with "Git tracked paths are not unique and sorted". Git returned its
index in strict raw pathname order, but the mutation driver converted each
name to `PathBuf` before comparing it. Component-wise path order disagrees
with Git at a file/directory stem boundary: `boot/limine.conf` correctly
precedes `boot/limine/BOOTX64.EFI` as index bytes but not as path components.
The validation therefore refused the repository it was meant to protect.

**Proof.** The real sweep on 05412f8 reproduced the pre-baseline refusal. An
extracted parser test failed under the old component comparison and accepts
the real file/directory boundary under byte comparison; a second test refuses
both duplicate and reversed inventories. Changing the strict comparison from
`<` to `!=` made only the reversed case red before restoration. That defect
remains mutation 1,358; the complete current sweep remains PW-001.
