# D-10 POSIX as a translation layer only; mlibc is the ports gateway

Status: Accepted, 2026-07-18. Amended by D-181.

**Decision.** POSIX lives entirely in userspace. Phase 9's mlibc sysdeps
make descriptors compatibility objects over native handles: they add
numbering, shared cursors, inheritance and root/cwd policy, and blocking
calls wrap ring submission and wait. A gated compat clone remains Phase 9
work, not a current kernel primitive. Native software never links this layer
(D-181).

**Rationale.** CPython and NetSurf (D-05) require POSIX and the native ABI
(D-06) is not; mlibc exists for hobby-OS ports.

**Consequences.** The Phase-5 ABI design doc must be written with mlibc's
sysdeps checklist open so no primitive is missing.
