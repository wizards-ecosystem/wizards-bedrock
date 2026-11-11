# D-418 Windows cleanup admits only its exact live executable

Status: Accepted, 2026-10-17. Amends D-382; closes the Windows completion defect in PW-016.

**Decision.** `cargo xtask clean` observes Cargo's status without initially
discarding it. Success is accepted normally. Failure is accepted only when the
canonical real target contains exactly one real `xtask` directory, that
directory contains exactly the platform's one expected `xtask` executable, the
leaf is a real regular file rather than a link or reparse point, and its
canonical identity equals the current process image. Any missing, additional,
foreign or redirected state preserves the failure. `clean --all` reaches the
VMware capability cleanup only after ordinary success or this exact residual
proof. The live image may remain until process exit; no background helper,
delete-on-reboot action or diagnostic-string exception is introduced.

Host fixtures cover success, a missing residual, the exact live image, extra
target and profile entries, a foreign current image, a directory at the
executable path and a file redirect whose outside sentinel remains unchanged.
Mutations 2,086 through 2,092 respectively bypass failure validation, weaken
each singleton, drop process-image identity and real-file classification, and
admit redirected target and profile directories; each names its exact red
witness. The real Windows command must report only the retained running image
and exit successfully, after which ordinary Cargo cleanup removes that unlocked
final file.

**Rationale.** On Windows Cargo returns access-denied while deleting the
`xtask.exe` executing the clean, so treating every nonzero status as failure
made the cleanup unusable there. The status alone does not prove what remains,
localized stderr is not filesystem authority, and a delayed deletion helper or
reboot-time deletion would expand process and persistence authority for a
deterministic one-file platform lock.
